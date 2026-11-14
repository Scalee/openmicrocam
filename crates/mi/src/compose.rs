//! The outputs' frames: every frame an encoder or
//! DISP (HDMI) gets is a canvas of its output's pool ([`crate::canvas`]). The feed's SCL port writes the live picture
//! straight into a view the module lends — the live window inside a free canvas, or the whole canvas when the live
//! picture is the frame — and the whole canvas goes to the pool's readers, each at its own rate: a reader is handed
//! a canvas when the canvas's capture time reaches its next one ([`Pace`]), so readers of one picture at different
//! rates read one pool; a canvas no reader takes goes back to the port as written. The still content — the pictures (overlapping as the scene has them, beside or over the
//! live picture), black where neither lies, and the `ui`'s patches over them — lies in each canvas beforehand, drawn by
//! GE; only the part of the port's write area that shows still content (pictures over the live window, the strips a
//! window widened to the port's 16-column grid adds) is put back after each write. With no live part visible
//! (covered) the port is off: the latest canvas brought up to date is handed again on the compositor's own clock, at
//! its fastest reader's rate.
//!
//! The still content reaches the output as the `ui`'s still set ([`StillSet`]): the pictures scaled to their windows
//! ([`clean`], one copy shared by every output's bake) and the menu over them as patches it bakes
//! (`mi::layer::bake`): NV12 rectangles of the still content with the menu over it as an RGN canvas shows it, never
//! written once made. A set comes through the output's [`PatchBox`], tagged with the geometry it was made for; a
//! compositor takes the sets of its own geometry, and before a canvas is handed on, GE copies into it what changed since
//! that canvas was last brought up to date — the windows whose picture changed, the still content where patches went
//! away, the patches that came — and puts the still content over the live window back after every write. The box hears
//! which set was in the last canvas handed, which tells the menu its revision is presented. Elements over the live
//! picture itself go through RGN on the port.
//!
//! A canvas is drawn into only while microcamd holds it: as a view it took from the port (until it is handed on), or taken
//! from the module ([`crate::canvas::take`]); the module gives the port and the readers only canvases nobody draws into.
//!
//! It is made by the pipeline thread and handed to the feed, which stops it with the feed (no `Drop` that
//! touches the hardware).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use microcam_core::desired::PictureId;
use microcam_core::pace::Pace;
use microcam_core::reconcile::{ChnPort, Module};
use microcam_core::{Rect, Size};
use mi_sys as ffi;

use crate::canvas::{self, Pool};
use crate::gfx::{self, Nv12};
use crate::picture::Frame;
use crate::{MiError, Result};

/// A patch: the menu baked over the still content at `rect` of the frame (`mi::layer::bake`), never written once made;
/// `rev` changes whenever its pixels would.
#[derive(Clone)]
pub struct Patch {
    pub rect: Rect,
    pub rev: u64,
    pub frame: Arc<Frame>,
}

/// The geometry a still set was made for: the frame's raster, its live window and its pictures' windows (bottom to
/// top). A compositor takes only a set of its own.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Tag {
    pub raster: Size,
    pub live: Rect,
    pub windows: Vec<Rect>,
}

/// What lies in a frame outside its visible live part, as the `ui` made it: a picture in each of the geometry's
/// windows (bottom to top) and the menu baked over them (the base patches, then the own patches on top). Its pictures
/// live while a box or a compositor holds the set.
#[derive(Clone, Default)]
pub struct StillSet {
    pub pictures: Vec<Clean>,
    pub patches: Vec<Patch>,
}

/// One output's still set: the latest the `ui` made (numbered, only the newest counts), and back from the compositor
/// the number of the set in the last canvas it handed on.
#[derive(Default)]
pub struct PatchBox {
    latest: Mutex<Option<(u64, Tag, Arc<StillSet>)>>,
    handed: AtomicU64,
    /// told when `handed` grows (the `ui` wakes)
    on_handed: OnceLock<Box<dyn Fn() + Send + Sync>>,
}

impl PatchBox {
    /// Set `seq` (above every earlier one), made for `tag`.
    pub fn put(&self, seq: u64, tag: Tag, set: Arc<StillSet>) {
        *self.latest.lock().unwrap() = Some((seq, tag, set));
    }

    /// No set any more (its output shows another's frames): what it held goes; the numbers handed stay.
    pub fn clear(&self) {
        *self.latest.lock().unwrap() = None;
    }

    /// The number of the set in the last canvas handed on (0: none yet).
    pub fn handed(&self) -> u64 {
        self.handed.load(Ordering::SeqCst)
    }

    /// `f` is told whenever a canvas with a newer set is handed on; once.
    pub fn on_handed(&self, f: Box<dyn Fn() + Send + Sync>) {
        let _ = self.on_handed.set(f);
    }

    /// The newest set if it was made for `tag` and is not `seen`.
    fn newer(&self, tag: &Tag, seen: u64) -> Option<(u64, Arc<StillSet>)> {
        let l = self.latest.lock().unwrap();
        l.as_ref().filter(|(seq, t, _)| *seq != seen && t == tag).map(|(seq, _, p)| (*seq, p.clone()))
    }

    fn handed_on(&self, seq: u64) {
        if seq > self.handed.fetch_max(seq, Ordering::SeqCst) {
            if let Some(f) = self.on_handed.get() {
                f();
            }
        }
    }
}

/// A picture's window of the frame and the picture in it, never changed: the decoded picture itself where a part of
/// it fits the window 1:1, else a frame of the window's size it is scaled into once. `at` is where the window's first
/// pixel lies in `frame`.
#[derive(Clone)]
pub struct Clean {
    pub window: Rect,
    pub frame: Arc<Frame>,
    pub at: (u32, u32),
}

impl Clean {
    /// Whether `o` lays the same pixels into the same window.
    fn same(&self, o: &Clean) -> bool {
        self.window == o.window && self.at == o.at && Arc::ptr_eq(&self.frame, &o.frame)
    }

    /// `r` (in the frame, inside the window) as a rectangle of `frame`.
    fn source(&self, r: Rect) -> Rect {
        Rect { x: self.at.0 + r.x - self.window.x, y: self.at.1 + r.y - self.window.y, ..r }
    }

    /// The clean as a bake reads it.
    pub fn still(&self) -> crate::layer::Still {
        crate::layer::Still { window: self.window, src: self.frame.nv12(), at: self.at }
    }
}

/// Pictures scaled to their windows, by the layout's decode, crop and window size: the first output's bake in the `ui`
/// makes one, the others take it; it lives while anything holds it (a still set, a baker).
type CleanKey = (PictureId, Rect, Size);
static CLEANS: OnceLock<Mutex<HashMap<CleanKey, Weak<Frame>>>> = OnceLock::new();

/// Picture `id` (its decode `frame`) shown as `crop` (source pixels) in `window`: the decode itself where its part is
/// 1:1 with the window, else the shared scaled copy, made by SCL dev 3 for `user` when nothing holds one.
pub fn clean(id: PictureId, frame: &Arc<Frame>, crop: Rect, window: Rect, user: crate::scl::User) -> std::result::Result<Clean, crate::layer::LayerError> {
    let part = frame.part(crop);
    if part.size() == window.size() {
        return Ok(Clean { window, frame: frame.clone(), at: (part.x, part.y) });
    }
    let key = (id, crop, window.size());
    let mut map = CLEANS.get_or_init(Default::default).lock().unwrap();
    map.retain(|_, w| w.strong_count() > 0);
    if let Some(f) = map.get(&key).and_then(Weak::upgrade) {
        return Ok(Clean { window, frame: f, at: (0, 0) });
    }
    let f = Frame::blank(window.size())?;
    let mut to = f.direct();
    (to.u32Width, to.u32Height) = (window.w, window.h);
    crate::scl::stretch(user, &frame.direct(), part, &to)?;
    let f = Arc::new(f);
    map.insert(key, Arc::downgrade(&f));
    Ok(Clean { window, frame: f, at: (0, 0) })
}

/// A photo during a recording, asked of the compositor of the pool the recording reads: the first finished canvas
/// programmed at microcam_rgn's frame `from` of `port` or later (None: the next) and holding patch set `seq` or a later one
/// handed to the JPEG encoder `venc`, a reader of that pool for one canvas, which encodes it beside the recording;
/// `reply` says when it is in (or why not).
pub struct Snap {
    pub venc: ChnPort,
    pub port: ChnPort,
    pub from: Option<u64>,
    /// the patch set baked for the photo's menu (0: none needed)
    pub seq: u64,
    pub deadline: Instant,
    pub reply: std::sync::mpsc::Sender<std::result::Result<(), String>>,
}

/// How long the start waits for each canvas to come back from the port and the readers.
const TAKE_WAIT: Duration = Duration::from_secs(1);
/// DISP keeps showing the last canvas it got until the next one comes, which a stopped feed never brings: the start
/// waits this long for such a canvas, then leaves it to be drawn whole when it is next written.
const DISP_HOLD_WAIT: Duration = Duration::from_millis(100);
/// No frame from the port for this long while it should be writing: a reader holds every canvas (it stopped).
pub const STALL: Duration = Duration::from_secs(2);

pub struct Composer {
    pool: Pool,
    size: Size,
    /// the port's write area in the canvases (the live window widened to the port's grid); None: covered, the port off
    view: Option<Rect>,
    /// the part of `view` showing still content, put back after every write: disjoint, each inside one picture's
    /// window or outside all of them
    repaint: Vec<Rect>,
    /// the frame outside the visible live part: where still content shows
    outside: Vec<Rect>,
    /// the geometry the sets must have been made for, where they come from, and the set the canvases are brought up to
    tag: Tag,
    patches: Arc<PatchBox>,
    shown: (u64, Arc<StillSet>),
    /// per canvas: what changed outside the live window since it was last brought up to date, the set it holds once
    /// nothing is left, and when a failed copy may be tried again
    dirty: Vec<Vec<Rect>>,
    holds: Vec<u64>,
    retry_at: Vec<Option<Instant>>,
    /// each reader's pace through the canvases, by its input port
    paces: Vec<(ChnPort, Pace)>,
    /// covered: the canvas handed again, the µs between frames (the fastest reader's) and the time the next one is due
    front: usize,
    interval: u64,
    due: Option<Instant>,
    last_frame: Instant,
    /// the reader refusing the hand-overs, and the first it refused since all readers last took one
    refused_since: Option<(Option<ChnPort>, Instant)>,
    stalled: bool,
    /// a photo asked, and whether the frames from its menu on are coming (microcam_rgn counted its frame `from` done)
    snap: Option<(Snap, bool)>,
    patch_failed: u64,
    handed: u64,
    /// canvases written that no reader's pace took
    over: u64,
    foreign: u64,
    failed: u64,
    /// DISP handed each canvas when the port is lent it, before the write (the pipeline allows it while
    /// the display lock holds): the newest lending seen, per canvas whether DISP has it already, how many went early
    early: bool,
    lent_seq: u32,
    early_handed: Vec<bool>,
    early_count: u64,
}

impl Composer {
    /// Start composing the frames of pool `pool` for its readers (`size` frames; `dst`, its own reader, at `fps`, the
    /// readers joining it at theirs): the still set from `patches`
    /// made for this geometry (`size`, the live window `live`, the pictures' `windows`) drawn into every canvas — its
    /// pictures bottom to top, black between, its patches over them; black alone while there is none — then the port
    /// lent views at `view` (None: covered), the `visible` part of the live window left to it.
    #[allow(clippy::too_many_arguments)]
    pub fn start(pool: u32, src: ChnPort, dst: ChnPort, size: Size, fps: u32, live: Rect, view: Option<Rect>, visible: &[Rect], windows: &[Rect], patches: Arc<PatchBox>) -> Result<Composer> {
        let fail = |e: String| {
            log::error!("compositor: {e}");
            MiError { call: "microcam_canvas", code: -1 }
        };
        canvas::rate(pool, dst, fps).map_err(fail)?;
        let pool = canvas::pool(pool).ok_or_else(|| fail(format!("no pool {pool}")))?;
        let tag = Tag { raster: size, live, windows: windows.to_vec() };
        let shown = patches.newer(&tag, 0).unwrap_or_default();
        let outside = microcam_core::frame::region(&[Rect::full(size)], if view.is_some() { visible } else { &[] }, windows);
        // the port lends nothing while the canvases are drawn; frames it wrote before wait in its queue holding their
        // canvases (a feed stopped with the port running): they go back
        canvas::view(pool.id, None).map_err(fail)?;
        let disp = pool.readers.iter().any(|r| r.port.module == Module::Disp);
        // canvases DISP still shows: drawn whole the first time they are brought up to date
        let mut held = Vec::new();
        'canvases: for k in 0..pool.canvases.len() {
            let t0 = Instant::now();
            while !canvas::take(pool.id, k).map_err(fail)? {
                give_back_queued(src);
                if disp && t0.elapsed() >= DISP_HOLD_WAIT {
                    held.push(k);
                    continue 'canvases;
                }
                if t0.elapsed() >= TAKE_WAIT {
                    return Err(fail(format!("canvas {k} of pool {} not back from the port and the readers in {} ms", pool.id, TAKE_WAIT.as_millis())));
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            let drawn = (|| {
                let to = pool.nv12(k);
                gfx::black_nv12(&to, Rect::full(size))?;
                for p in &shown.1.pictures {
                    gfx::copy_nv12(&p.frame.nv12(), p.source(p.window), &to, p.window.x, p.window.y)?;
                }
                for t in &shown.1.patches {
                    gfx::copy_nv12(&t.frame.nv12(), Rect::full(t.rect.size()), &to, t.rect.x, t.rect.y)?;
                }
                wait()
            })();
            canvas::give(pool.id, k).map_err(fail)?;
            drawn?;
        }
        let repaint = match view {
            Some(v) => microcam_core::frame::region(&[v], visible, windows),
            None => Vec::new(),
        };
        if let Some(v) = view {
            canvas::view(pool.id, Some((v.x, v.y))).map_err(fail)?;
        }
        log::info!(
            "compositor: pool {} ({} canvases, {} still shown by DISP), {}; set {}: {} pictures of {} windows, {} patches; {} rectangles put back per frame",
            pool.id,
            pool.canvases.len(),
            held.len(),
            view.map_or("covered".to_string(), |v| format!("the port writes {}x{}+{}+{}", v.w, v.h, v.x, v.y)),
            shown.0,
            shown.1.pictures.len(),
            windows.len(),
            shown.1.patches.len(),
            repaint.len()
        );
        let n = pool.canvases.len();
        let mut dirty = vec![Vec::new(); n];
        let mut holds = vec![shown.0; n];
        for &k in &held {
            dirty[k] = outside.clone();
            holds[k] = 0;
        }
        let front = (0..n).find(|k| !held.contains(k)).unwrap_or(0);
        Ok(Composer {
            pool,
            size,
            view,
            repaint,
            outside,
            tag,
            patches,
            holds,
            shown,
            dirty,
            retry_at: vec![None; n],
            paces: Vec::new(),
            front,
            interval: 1_000_000 / fps.max(1) as u64,
            due: view.is_none().then(Instant::now),
            last_frame: Instant::now(),
            refused_since: None,
            stalled: false,
            snap: None,
            patch_failed: 0,
            handed: 0,
            over: 0,
            foreign: 0,
            failed: 0,
            early: false,
            lent_seq: 0,
            early_handed: vec![false; n],
            early_count: 0,
        })
    }

    /// A newer still set marked in every canvas where it changed.
    fn tick_patches(&mut self) {
        if let Some(latest) = self.patches.newer(&self.tag, self.shown.0) {
            let changed = changed(&self.shown.1, &latest.1);
            for d in &mut self.dirty {
                for r in &changed {
                    mark(d, *r, &self.outside);
                }
            }
            self.shown = latest;
        }
    }

    /// Covered: when the next frame is due (the feed's own clock); None: the port is the clock.
    pub(crate) fn due(&self) -> Option<Instant> {
        self.due
    }

    /// Between frames: a newer patch set marked where it changed; covered and due, the frame handed. Once, when a
    /// reader is lost, that reader (None: not known): it refused every canvas for [`crate::feed::LOST_AFTER`], or the
    /// port has brought no frame for [`STALL`] (a reader holds every canvas — an encoder among the readers is taken for
    /// it: DISP lets its canvases go on its own clock).
    pub(crate) fn tick(&mut self) -> Option<Option<ChnPort>> {
        self.tick_patches();
        if let Some((who, t)) = self.refused_since.filter(|_| !self.stalled) {
            if t.elapsed() >= crate::feed::LOST_AFTER {
                self.stalled = true;
                log::error!("compositor: pool {}: a reader refuses every canvas", self.pool.id);
                return Some(who);
            }
        }
        if let Some(due) = self.due {
            if Instant::now() >= due {
                self.covered_frame();
                // a late wake does not send a burst: the next one is a period from now
                self.due = Some(due.max(Instant::now() - Duration::from_micros(self.interval)) + Duration::from_micros(self.interval));
            }
            return None;
        }
        if !self.stalled && self.last_frame.elapsed() >= STALL {
            self.stalled = true;
            log::error!("compositor: pool {}: no frame from the port for {} s: a reader holds every canvas", self.pool.id, STALL.as_secs());
            let readers: Vec<ChnPort> = canvas::pool(self.pool.id).map(|p| p.readers.iter().map(|r| r.port).collect()).unwrap_or_default();
            return Some(readers.iter().copied().find(|r| r.module == Module::Venc).or(readers.first().copied()));
        }
        None
    }

    /// One live frame (`h`, a view taken from the feed's port): when a reader's pace takes it, the still content put
    /// back where the port wrote over it, the canvas brought up to date, then handed to those readers; the view goes
    /// back to the port after.
    pub(crate) fn frame(&mut self, h: ffi::MI_SYS_BUF_HANDLE, info: &ffi::MI_SYS_BufInfo_t) {
        self.last_frame = Instant::now();
        self.stalled = false;
        // SAFETY: the frame view of a frame buffer the library just handed out.
        let d = unsafe { &info.__bindgen_anon_1.stFrameData };
        let got = Size::new(d.u16Width as u32, d.u16Height as u32);
        let k = self.pool.which(d.phyAddr[0]);
        let (Some(k), Some(v)) = (k, self.view) else {
            // a frame of the port's own buffers (from before the pool), or one written while covered
            unsafe { ffi::MI_SYS_ChnOutputPortPutBufPa(h) };
            self.foreign += 1;
            return;
        };
        if got != v.size() {
            // a frame of another size on its way out: not the layout this canvas holds
            unsafe { ffi::MI_SYS_ChnOutputPortPutBufPa(h) };
            self.foreign += 1;
            return;
        }
        let pts = info.u64Pts;
        // DISP has it already when it was handed it at the lending: only while the display lock holds, the sensor at
        // the display's rate, so DISP's pace takes every canvas
        let early = std::mem::take(&mut self.early_handed[k]);
        let takers = self.takers(pts);
        if takers.is_empty() && !early {
            unsafe { ffi::MI_SYS_ChnOutputPortPutBufPa(h) };
            self.over += 1;
            return;
        }
        let drawn = {
            self.bring_up(k);
            let to = self.pool.nv12(k);
            let failed = patch(&self.shown.1, &self.repaint, &to);
            let r = wait();
            match failed.into_iter().next() {
                Some((_, e)) => Err(e),
                None => r,
            }
        };
        let r = drawn
            .map_err(|e| canvas::Refused { reader: None, error: e.to_string() })
            .and_then(|_| canvas::hand_to(self.pool.id, k, self.size, pts, |r| takers.contains(&r) && !(early && r.module == Module::Disp)));
        if r.is_ok() {
            self.patches.handed_on(self.holds[k]);
            // a photo: the canvas to its encoder too
            self.snapshot(k, pts);
        }
        unsafe { ffi::MI_SYS_ChnOutputPortPutBufPa(h) };
        self.count(r);
    }

    /// A photo without a recording: the encoder (the photo's own) gets one canvas, with the menu as the `ui` baked it in
    /// patch set `seq` or a later one and, when the port writes (not covered), the live picture of the first frame of
    /// `src` programmed at microcam_rgn's frame `from` or later (None: the newest). The compositor runs for nothing else.
    pub fn shoot(&mut self, src: ChnPort, from: Option<u64>, seq: u64, timeout_ms: u32) -> std::result::Result<(), String> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64);
        while self.shown.0 < seq {
            if Instant::now() >= deadline {
                return Err(format!("the photo's menu (patch set {seq}) not in {timeout_ms} ms"));
            }
            std::thread::sleep(Duration::from_millis(5));
            self.tick_patches();
        }
        let Some(v) = self.view else {
            // covered: a canvas taken from the module, brought up to date, handed
            let k = self.front;
            let t0 = Instant::now();
            while !canvas::take(self.pool.id, k).map_err(|e| e.to_string())? {
                if t0.elapsed() >= TAKE_WAIT {
                    return Err(format!("canvas {k} not back from the readers"));
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            self.bring_up(k);
            let r = wait();
            canvas::give(self.pool.id, k).map_err(|e| e.to_string())?;
            r.map_err(|e| e.to_string())?;
            return canvas::hand(self.pool.id, k, self.size, crate::sys::now_pts()).map_err(|e| e.to_string());
        };
        let left = deadline.saturating_duration_since(Instant::now()).as_millis() as i32;
        let (h, info) = crate::shot::take(src, v.size(), from, left)?;
        // SAFETY: the frame view of a frame buffer the library handed out.
        let k = self.pool.which(unsafe { info.__bindgen_anon_1.stFrameData.phyAddr[0] });
        let r = (|| {
            let k = k.ok_or("the frame is not in the pool's canvases")?;
            self.bring_up(k);
            let failed = patch(&self.shown.1, &self.repaint, &self.pool.nv12(k));
            wait().map_err(|e| e.to_string())?;
            if let Some((r, e)) = failed.into_iter().next() {
                return Err(format!("{r:?}: {e}"));
            }
            canvas::hand(self.pool.id, k, self.size, info.u64Pts).map_err(|e| e.to_string())
        })();
        unsafe { ffi::MI_SYS_ChnOutputPortPutBufPa(h) };
        r
    }

    /// A photo is asked of these frames ([`Snap`]); one asked before it is given up.
    pub(crate) fn snap(&mut self, s: Snap) {
        let armed = s.from.is_none();
        if let Some((old, _)) = self.snap.replace((s, armed)) {
            let _ = old.reply.send(Err("another photo was asked".into()));
        }
    }

    /// Canvas `k` (finished, microcamd's) into the photo's encoder when a photo waits for it.
    fn snapshot(&mut self, k: usize, pts: u64) {
        let Some((s, armed)) = self.snap.as_mut() else { return };
        if Instant::now() >= s.deadline {
            let (s, _) = self.snap.take().unwrap();
            let _ = s.reply.send(Err("no frame with the photo's menu in time".into()));
            return;
        }
        if !*armed {
            // a frame queued when microcam_rgn counts `from` done may still be older: the next one is taken
            match crate::rgn::frames(s.port) {
                Ok((_, done)) => *armed = done >= s.from.unwrap_or(0),
                Err(e) => {
                    let (s, _) = self.snap.take().unwrap();
                    let _ = s.reply.send(Err(e.to_string()));
                }
            }
            return;
        }
        if self.holds[k] < s.seq {
            return;
        }
        let (s, _) = self.snap.take().unwrap();
        let _ = s.reply.send(canvas::hand_once(self.pool.id, k, s.venc, self.size, pts));
    }

    /// Covered: a canvas other than the front brought up to date when the patches changed (taken from the module, so
    /// no reader nor anything else reads it meanwhile), made the front once whole; the front handed again. The first
    /// one the module gives: a reader keeps a canvas past the hand-over of the next front (DISP about two frame
    /// periods), so the front before this one is still held while the one before that is free. The front goes to the
    /// readers whose pace takes it; the clock follows the fastest reader's rate.
    fn covered_frame(&mut self) {
        let n = self.pool.canvases.len();
        let taken = (0..n).filter(|&k| k != self.front && !self.dirty[k].is_empty()).find(|&k| canvas::take(self.pool.id, k).unwrap_or(false));
        if let Some(k) = taken {
            self.bring_up(k);
            if let Err(e) = wait() {
                // whether the copies landed is unknown: the canvas is brought up to date whole again
                log::warn!("compositor: {e}");
                self.dirty[k] = self.outside.clone();
            }
            let _ = canvas::give(self.pool.id, k);
        }
        if !self.dirty[self.front].is_empty() {
            if let Some(k) = (0..n).find(|&k| self.dirty[k].is_empty()) {
                self.front = k;
            }
        }
        let pts = crate::sys::now_pts();
        let takers = self.takers(pts);
        if let Some(fps) = canvas::pool(self.pool.id).and_then(|p| p.readers.iter().map(|r| r.fps).max()).filter(|f| *f > 0) {
            self.interval = 1_000_000 / fps as u64;
        }
        if takers.is_empty() {
            self.over += 1;
            return;
        }
        let r = canvas::hand_to(self.pool.id, self.front, self.size, pts, |r| takers.contains(&r));
        if r.is_ok() {
            self.patches.handed_on(self.holds[self.front]);
        }
        self.count(r);
    }

    /// The readers whose pace takes the canvas of `pts`, each at its rate now: a reader that left loses its pace, one
    /// that came gets one; a reader whose rate is not known yet takes none.
    fn takers(&mut self, pts: u64) -> Vec<ChnPort> {
        let readers = canvas::pool(self.pool.id).map(|p| p.readers).unwrap_or_default();
        self.paces.retain(|(port, _)| readers.iter().any(|r| r.port == *port));
        let mut takers = Vec::new();
        for r in readers.iter().filter(|r| r.fps > 0) {
            let pace = match self.paces.iter_mut().find(|(port, _)| *port == r.port) {
                Some((_, p)) => p,
                None => {
                    self.paces.push((r.port, Pace::new(r.fps)));
                    &mut self.paces.last_mut().unwrap().1
                }
            };
            pace.set(r.fps);
            if pace.take(pts) {
                takers.push(r.port);
            }
        }
        takers
    }

    pub(crate) fn pool_id(&self) -> u32 {
        self.pool.id
    }

    /// Whether DISP is handed each canvas at its lending (the pipeline's word).
    pub(crate) fn set_early(&mut self, on: bool) {
        if on != self.early {
            log::info!("compositor: pool {}: DISP handed each canvas {} ({} so far before the write)", self.pool.id, if on { "when the port is lent it" } else { "once written" }, self.early_count);
        }
        self.early = on;
    }

    pub(crate) fn early(&self) -> bool {
        self.early
    }

    /// The port was lent canvas `l.canvas` (the module's newest lending of this pool): DISP gets it now, before the port
    /// writes it, when early is on and the frame needs nothing after the write — the live picture is the whole frame,
    /// no picture or patch over it. Otherwise it goes to DISP with the rest once written.
    pub(crate) fn lent(&mut self, l: canvas::Lending) {
        if l.seq == self.lent_seq || l.canvas >= self.early_handed.len() {
            return;
        }
        self.lent_seq = l.seq;
        let k = l.canvas;
        self.early_handed[k] = false;
        let whole = self.view == Some(Rect::full(self.size)) && self.repaint.is_empty() && self.tag.windows.is_empty() && self.dirty[k].is_empty();
        if !self.early || !whole || !self.pool.readers.iter().any(|r| r.port.module == Module::Disp) {
            return;
        }
        match canvas::hand_to(self.pool.id, k, self.size, crate::sys::now_pts(), |r| r.module == Module::Disp) {
            Ok(()) => {
                self.early_handed[k] = true;
                self.early_count += 1;
            }
            Err(e) => log::debug!("compositor: canvas {k} not handed to DISP before the write: {e}"),
        }
    }

    fn count(&mut self, r: std::result::Result<(), canvas::Refused>) {
        match r {
            Ok(()) => {
                self.handed += 1;
                self.refused_since = None;
            }
            Err(e) => {
                self.failed += 1;
                if self.refused_since.is_none_or(|(who, _)| who != e.reader) {
                    self.refused_since = Some((e.reader, Instant::now()));
                }
                if self.failed == 1 || self.failed % 1000 == 0 {
                    log::warn!("compositor: {e} ({} so far)", self.failed);
                }
            }
        }
    }

    /// Bring canvas `k` (microcamd's) up to date where the patches changed; once nothing is left it holds the set shown.
    /// The live picture goes out every frame whatever the patches do: a copy that fails leaves this canvas's menu partly
    /// old (mixed) while its live part is fresh, and the failed part is tried again at the canvas's turn [`RETRY`]
    /// later, so a copy that keeps failing does not take the engine's time every frame.
    fn bring_up(&mut self, k: usize) {
        if self.dirty[k].is_empty() {
            self.holds[k] = self.shown.0;
            return;
        }
        if self.retry_at[k].is_some_and(|t| Instant::now() < t) {
            return;
        }
        let failed = patch(&self.shown.1, &self.dirty[k], &self.pool.nv12(k));
        self.retry_at[k] = (!failed.is_empty()).then(|| Instant::now() + RETRY);
        self.dirty[k] = failed.iter().map(|(r, _)| *r).collect();
        if self.dirty[k].is_empty() {
            self.holds[k] = self.shown.0;
        }
        for (r, e) in failed {
            self.patch_failed += 1;
            if self.patch_failed <= 8 || self.patch_failed % 1000 == 0 {
                log::warn!("compositor: patch {r:?}: {e} ({} so far)", self.patch_failed);
            }
        }
    }

    /// The port stops being lent views (the canvases stay the pool's).
    pub fn stop(mut self) -> std::result::Result<(), String> {
        if let Some((s, _)) = self.snap.take() {
            let _ = s.reply.send(Err("the recording's feed stopped".into()));
        }
        let _ = canvas::view(self.pool.id, None);
        log::info!("compositor stopped: {} handed ({} to DISP before the write), {} over every reader's rate, {} foreign frames given back, {} failed", self.handed, self.early_count, self.over, self.foreign, self.failed);
        Ok(())
    }
}

/// Where the still sets `old` and `new` of one geometry differ: the windows whose picture changed, the places of
/// patches that went, came or changed.
fn changed(old: &StillSet, new: &StillSet) -> Vec<Rect> {
    let n = old.pictures.len().max(new.pictures.len());
    let mut out: Vec<Rect> = (0..n)
        .filter_map(|i| match (old.pictures.get(i), new.pictures.get(i)) {
            (Some(a), Some(b)) if a.same(b) => None,
            (a, b) => a.or(b).map(|c| c.window),
        })
        .collect();
    let missing = |a: &[Patch], b: &[Patch]| a.iter().filter(|t| !b.iter().any(|u| u.rect == t.rect && u.rev == t.rev)).map(|t| t.rect).collect::<Vec<_>>();
    out.extend(missing(&old.patches, &new.patches));
    out.extend(missing(&new.patches, &old.patches));
    out.dedup();
    out
}

/// How long a buffer whose patch copy failed waits before the next try.
const RETRY: Duration = Duration::from_secs(1);

/// More places to bring up to date than this, and a canvas is brought up to date wherever still content shows.
const MAX_DIRTY: usize = 32;

/// `r` added to a canvas's places to bring up to date, once; past [`MAX_DIRTY`] they become `outside` whole (the frame
/// outside the visible live part: never the live picture the port just wrote).
fn mark(dirty: &mut Vec<Rect>, r: Rect, outside: &[Rect]) {
    if dirty.contains(&r) {
        return;
    }
    dirty.push(r);
    if dirty.len() > MAX_DIRTY {
        *dirty = outside.to_vec();
    }
}

/// Bring `dirty` of canvas `to` to the still set: black where no picture lies, its pictures (bottom to top) and its
/// patches over them, by GE (1:1; queued, the caller waits). Every copy is tried; the ones the engine refused are
/// returned with their place.
fn patch(set: &StillSet, dirty: &[Rect], to: &Nv12) -> Vec<(Rect, MiError)> {
    let (clean, patches) = (&set.pictures, &set.patches);
    let mut failed = Vec::new();
    for d in dirty {
        if !clean.iter().any(|c| c.window.contains(*d)) {
            if let Err(e) = gfx::black_nv12(to, *d) {
                failed.push((*d, e));
            }
        }
        for c in clean {
            let i = d.intersect(c.window);
            if !i.is_empty() {
                if let Err(e) = gfx::copy_nv12(&c.frame.nv12(), c.source(i), to, i.x, i.y) {
                    failed.push((i, e));
                }
            }
        }
    }
    for t in patches.iter().filter(|t| dirty.iter().any(|d| !d.intersect(t.rect).is_empty())) {
        for d in dirty {
            // only inside the dirty places: a patch may reach over the live picture's edge
            let i = d.intersect(t.rect);
            if i.is_empty() {
                continue;
            }
            let src = Rect { x: i.x - t.rect.x, y: i.y - t.rect.y, ..i };
            if let Err(e) = gfx::copy_nv12(&t.frame.nv12(), src, to, i.x, i.y) {
                failed.push((i, e));
            }
        }
    }
    failed
}

/// Every frame `src` has queued for us, back to it unused.
fn give_back_queued(src: ChnPort) {
    let mut port = crate::sys::chn_port(src);
    loop {
        // SAFETY: all-zero is a valid BufInfo (plain data).
        let mut info: ffi::MI_SYS_BufInfo_t = unsafe { std::mem::zeroed() };
        let mut h: ffi::MI_SYS_BUF_HANDLE = 0;
        if unsafe { ffi::MI_SYS_ChnOutputPortGetBufPa(&mut port, &mut info, &mut h) } != 0 {
            return;
        }
        unsafe { ffi::MI_SYS_ChnOutputPortPutBufPa(h) };
    }
}

/// Everything queued on GE done; a wait that fails leaves the engine lost (`gfx::mark_lost`).
fn wait() -> Result<()> {
    gfx::wait().inspect_err(|_| gfx::mark_lost())
}
