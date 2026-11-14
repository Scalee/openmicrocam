//! The `ui`: the one state every output's renderer reads — the scene the menu program sent
//! and where each output is — and the thread that draws it. The scene is cut into items per output and rasterised once
//! into tiles shared by all outputs (`microcam_draw::render`, `mi::layer::Tiles`); each output's layer is brought to its
//! items by the GE (`mi::layer::Composer`): on the HDMI, USB and recording ports, RGN regions over the parts of the
//! frame the items take (one GWIN each, `microcam_draw::layer::parts`), drawn into the canvas the driver lends and shown
//! from the next frame on. A photo is a frame of the media port with its layer drawn for the scene of the press, held
//! there until the photo is taken ([`Ui::photo`]). Every still frame's menu over the pictures and black of its frame is
//! baked once from the same tiles into patches ([`Bake`]), which the compositor of every output showing it lays into
//! its frames ([`Out`]). An output's frame changes at one moment: once its still set is whole — every picture of the
//! frame decoded, the base baked — its layout goes to the pipeline; until then, and until the pipeline built it, the
//! output shows what it showed.
//!
//! Writers (control connections, the pipeline) replace parts of the state in [`Ui`]'s mailbox and wake the thread; it
//! draws the latest state only, so a burst of scenes costs one drawing, not a queue. The RGN regions sit on the
//! pipeline's ports, so the pipeline says when an output's layer may be there ([`Ui::attached`]) and has them taken off
//! ([`Ui::release_layer`]) before a step takes the port away, which waits while this thread draws into them.

use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use microcam_core::desired::PictureId;
use microcam_core::edit::{apply, Op};
use microcam_core::frame;
use microcam_core::policy::{Frames, Layout};
use microcam_core::overlay::{Attached, RgnOutput};
use microcam_core::controls::{ordered, Code, Held, Holder, Refusal};
use microcam_core::protocol::{Event, Output, OutputInfo, ResCode, ResState};
use microcam_core::reconcile::ChnPort;
use microcam_core::scene::Scene;
use microcam_core::ui::OutputUi;
use microcam_core::wish::Wish;
use microcam_core::{Rect, Size};
use microcam_draw::layer::{bake_areas, clusters, covers, on_live};
use microcam_draw::render::{shown, Fonts, Item};
use mi::compose::{Clean, Patch, PatchBox, StillSet, Tag};
use mi::gfx::{Fmt, Surface};
use mi::layer::{Charge, Composer, Scratch, Sink, Stats, Tiles, Use};

use crate::followers::{Client, Followers};
use crate::pipeline::Msg;
use crate::res::{Change, Resources};

/// The outputs whose `ui` goes into an RGN layer: every one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RgnId {
    Usb,
    /// the media encoder recording (a photo is not an RGN output: [`Ui::photo`])
    Media,
    /// DISP's frames: the HDMI pool's canvases, its port SCL port 0 (or HDMI's upscaler when zoomed)
    Hdmi,
}

// an output's patch box is the Hal's by its pool and the `ui`'s by its id: the same index
const _: () = {
    use microcam_core::reconcile::topo::{POOL_HDMI, POOL_MEDIA, POOL_USB};
    assert!(RgnId::Usb as u32 == POOL_USB && RgnId::Media as u32 == POOL_MEDIA && RgnId::Hdmi as u32 == POOL_HDMI);
};

impl RgnId {
    pub const ALL: [RgnId; 3] = [RgnId::Usb, RgnId::Media, RgnId::Hdmi];

    pub fn name(self) -> &'static str {
        match self {
            RgnId::Usb => "usb",
            RgnId::Media => "media",
            RgnId::Hdmi => "hdmi",
        }
    }

    pub fn output(self) -> Output {
        match self {
            RgnId::Usb => Output::Usb,
            RgnId::Media => Output::Media,
            RgnId::Hdmi => Output::Hdmi,
        }
    }
}

/// After a drawing could not be put in place, the next try even if nothing changes.
const RETRY: Duration = Duration::from_secs(1);
/// The GOP's windows on a port (the driver's limit, the hardware's count): an output's layer is at most this many regions.
const GWINS: usize = 8;
/// The first of each RGN output's region handles, by [`RgnId`] (handles are global to the system, microcam_rgn takes up to
/// 1024; the vendor's tools used single digits); each output takes [`HANDLES`] from there.
const HANDLE_BASE: [u32; 3] = [64, 80, 104];
const HANDLES: u32 = 16;
/// A port's covers (the driver's limit), and the first of each RGN output's cover handles, by [`RgnId`].
const COVERS: usize = 4;
const COVER_BASE: [u32; 3] = [96, 100, 120];

#[derive(Default)]
struct State {
    /// the menu's scene and its revision; None while no menu holds it or it is empty, as taken (revision 0)
    scene: Option<(u64, Arc<Scene>)>,
    /// each RGN output as built and the generation the pipeline named it by (None: not built, or its layer cannot be
    /// put on its port), by [`RgnId`]
    rgn: [Option<(u64, RgnOutput)>; 3],
    /// each output as the menu is told of it, from the plan built (the same as `status`), by [`RgnId`]
    infos: [Option<OutputInfo>; 3],
    /// an output built whose layer cannot be put on its port, and why: the menu hears `not-shown`
    unplaced: [Option<String>; 3],
    /// the photo being taken: the media layer shows its scene, not the current one
    photo: Option<PhotoJob>,
    /// bumped by every change, so the thread knows there is something to draw
    version: u64,
    /// bumped when an output reached what was drawn (a canvas handed): a round only to tell `presented`,
    /// not held to the frame pace
    notices: u64,
    /// the pipeline refused the frames' layout last sent, and why: the menu hears it once (the pipeline keeps the
    /// layout and builds it when it can)
    refused: Option<Refusal>,
    /// the video needs microcamd's share: what nothing shows goes, then what the precedence gave way
    give_back: Option<GiveBack>,
}

impl State {
    /// The outputs the scene is shown on, as the menu is told of them: HDMI first.
    fn outputs(&self) -> Vec<OutputInfo> {
        [RgnId::Hdmi, RgnId::Usb, RgnId::Media].into_iter().filter_map(|id| self.infos[id as usize].clone()).collect()
    }
}

/// A photo's `ui`: the scene as it was when the photo was asked, drawn into the media layer.
pub struct PhotoJob {
    pub id: u64,
    pub scene: Arc<Scene>,
}

/// The shared side of the `ui`: what control and the pipeline write, and each RGN output's layer as the pipeline put
/// it on its port.
pub struct Ui {
    state: Mutex<State>,
    wake: Condvar,
    /// each RGN output's layer: held by this thread while it draws into it, by the pipeline while it has it taken off
    layers: [Mutex<Layer>; 3],
    pipeline: Mutex<Sender<Msg>>,
    /// the menu's resources
    pub res: Arc<Resources>,
    /// the menu, which owns the scene, and the watchers: what the camera's state is told to
    pub followers: Arc<Followers>,
    /// what the video holds of the media heap, by what turns it off or lowers it (the pipeline's, after each message)
    video: Mutex<Vec<Holder>>,
}

impl Ui {
    fn change(&self, f: impl FnOnce(&mut State) -> bool) {
        let mut s = self.state.lock().unwrap();
        let had = s.scene.is_some();
        let outputs = s.outputs();
        if f(&mut s) {
            s.version += 1;
            self.wake.notify_one();
            let now = s.outputs();
            if now != outputs {
                self.followers.tell(&Event::Outputs { outputs: now });
            }
        }
        // the pipeline puts layers on the ports only while there is a scene
        if had != s.scene.is_some() {
            let _ = self.pipeline.lock().unwrap().send(Msg::UiScene);
        }
    }

    /// An output reached what was drawn, or microcamd's share freed what a form waited for: the thread looks at once.
    pub fn notice(&self) {
        self.state.lock().unwrap().notices += 1;
        self.wake.notify_one();
    }

    /// `client` follows the camera from now on: as the menu (`take` given: holding an empty scene at revision 0; refused
    /// while another holds it, unless `take`), or as a watcher (None). Its reply — `reply` made from the outputs, with the
    /// snapshot — is queued under the followers' lock with the connection subscribed in the same section, and the
    /// outputs read under this one, which their event goes out under: no event falls between the snapshot and those
    /// after it.
    pub fn subscribe(&self, client: Client, take: Option<bool>, reply: impl FnOnce(Vec<OutputInfo>) -> String) -> Result<(), String> {
        let Some(take) = take else {
            let s = self.state.lock().unwrap();
            self.followers.watch(client, || reply(s.outputs()));
            return Ok(());
        };
        let mut r = Ok(());
        let mut lost = false;
        self.change(|s| {
            match self.followers.hello(client, take, || reply(s.outputs())) {
                Ok(l) => lost = l,
                Err(e) => {
                    r = Err(e);
                    return false;
                }
            }
            // a menu's resources are its own; its next request comes after this one
            self.res.clear();
            s.scene.take().is_some()
        });
        if lost {
            self.owner_lost();
        }
        r
    }

    /// The scene taken from its owner (`reset`): it hears `lost`, and its commits and `keep` are refused from then on;
    /// the next `hello` starts from an empty scene.
    pub fn take_scene(&self) {
        let mut lost = false;
        self.change(|s| {
            lost = self.followers.drop_menu();
            if lost {
                self.res.clear();
                s.scene = None;
            }
            lost
        });
        if lost {
            self.owner_lost();
        }
    }

    /// The scene's owner is gone: the player stops (every output shows the camera); a recording, the settings and the
    /// locks go on.
    fn owner_lost(&self) {
        let _ = self.pipeline.lock().unwrap().send(Msg::Wish(Wish::PlayStop, None));
    }

    /// Whether `conn` is the menu and the scene at `base` (the owner changes under this lock).
    fn check_base(&self, s: &State, conn: u64, base: u64) -> Result<(), String> {
        match self.followers.menu() {
            Some(m) if m.conn == conn => {
                let revision = s.scene.as_ref().map_or(0, |(r, _)| *r);
                if revision == base {
                    Ok(())
                } else {
                    Err(format!("base {base}, the scene is at revision {revision}"))
                }
            }
            Some(_) => Err("another connection is the menu".into()),
            None => Err("hello as the menu first".into()),
        }
    }

    /// The menu `conn`'s commit: `ops` on revision `base`, the new revision or why not. `accepted` runs with the new
    /// revision before the thread can draw it, so the menu hears of the acceptance before any `presented` for it.
    pub fn commit(&self, conn: u64, base: u64, ops: &[Op], accepted: impl FnOnce(u64)) -> Result<u64, String> {
        let current = {
            let s = self.state.lock().unwrap();
            self.check_base(&s, conn, base)?;
            s.scene.as_ref().map(|(_, sc)| sc.clone())
        };
        // applied and checked outside the lock: only this connection commits, a `take` meanwhile is caught below
        let new = apply(current.as_deref().unwrap_or(&Scene::empty()), ops)?;
        new.validate().map_err(|e| e.to_string())?;
        if let Some(id) = new.resources().into_iter().find(|id| !self.res.known(id)) {
            return Err(format!("no resource '{id}'"));
        }
        let mut r = Err(String::new());
        self.change(|s| {
            if let Err(e) = self.check_base(s, conn, base) {
                r = Err(e);
                return false;
            }
            let rev = base + 1;
            s.scene = Some((rev, Arc::new(new)));
            accepted(rev);
            r = Ok(rev);
            true
        });
        r
    }

    /// The control connection `conn` closed: it follows nothing more; if it was the menu, the scene goes from every
    /// output and the player stops.
    pub fn gone(&self, conn: u64) {
        let mut lost = false;
        self.change(|s| {
            lost = self.followers.gone(conn);
            if lost {
                s.scene = None;
                self.res.clear();
            }
            lost
        });
        if lost {
            self.owner_lost();
        }
    }

    /// A resource changed: the menu hears of a new state, and the outputs are drawn again.
    pub fn res_changed(&self, c: Change) {
        let c = match c {
            Change::State(id, ResState::Failed { code: ResCode::Resources, error, holders }) if holders.is_empty() => Change::State(id, ResState::Failed { code: ResCode::Resources, error, holders: self.holders() }),
            c => c,
        };
        self.change(|_| {
            if let Change::State(id, state) = c {
                self.followers.to_menu(&Event::Resource { id, state });
            }
            true
        });
    }

    pub fn has_scene(&self) -> bool {
        self.state.lock().unwrap().scene.is_some()
    }

    /// `id` as built: its RGN output (None: none, or its layer cannot be put on its port — `unplaced` says why) and
    /// what the menu is told of it.
    pub fn rgn(&self, id: RgnId, o: Option<(u64, RgnOutput)>, info: Option<OutputInfo>, unplaced: Option<String>) {
        self.change(|s| {
            let k = id as usize;
            let changed = s.rgn[k] != o || s.infos[k] != info || s.unplaced[k] != unplaced;
            (s.rgn[k], s.infos[k], s.unplaced[k]) = (o, info, unplaced);
            changed
        });
    }

    /// What the video holds now, as the pipeline built it.
    pub fn video_holders(&self, v: Vec<Holder>) {
        *self.video.lock().unwrap() = v;
    }

    /// What holds the camera's resources now, for a refusal for lack of them: the video by what turns it off or lowers
    /// it, each of the menu's pictures by its id, and the rest of microcamd's hold as the menu's drawing; the most first.
    pub fn holders(&self) -> Vec<Holder> {
        let mut v = self.video.lock().unwrap().clone();
        let pictures = self.res.held();
        let drawing = mi::layer::firm().saturating_sub(pictures.iter().map(|(_, b)| b).sum());
        v.extend(pictures.into_iter().map(|(id, b)| Holder { what: Held::Picture { id }, bytes: Some(b) }));
        v.push(Holder { what: Held::Drawing, bytes: Some(drawing) });
        ordered(v)
    }

    /// The pipeline allows `id`'s layer on its port: this thread may put regions there.
    pub fn attached(&self, id: RgnId, a: Attached) {
        self.layers[id as usize].lock().unwrap().grant = Some(a);
        self.change(|_| true);
    }

    /// `id`'s regions taken off their port: waits while this thread draws into them, and from then on it puts none there
    /// until [`Self::attached`]. False when the 2D engine is lost: it may still be writing them, so they stay.
    pub fn release_layer(&self, id: RgnId) -> bool {
        let mut l = self.layers[id as usize].lock().unwrap();
        l.grant = None;
        if mi::gfx::lost() {
            return false;
        }
        l.take_off(id);
        true
    }

    /// Whether `id`'s layer shows nothing on its port: no region or cover shown (none there once taken off).
    fn layer_bare(&self, id: RgnId) -> bool {
        let l = self.layers[id as usize].lock().unwrap();
        l.parts.iter().all(|p| !p.shown) && l.covers.iter().all(|c| c.shows.is_none())
    }

    /// The tiles `id`'s regions show.
    fn layer_keys(&self, id: RgnId) -> Vec<u64> {
        self.layers[id as usize].lock().unwrap().parts.iter().filter(|p| p.shown).flat_map(|p| p.composer.shown().iter().map(|i| i.key)).collect()
    }

    /// The RGN outputs' hardware windows as they are now, one line each (`microcam ui-windows`).
    pub fn windows(&self) -> Vec<String> {
        let mut out = Vec::new();
        for id in RgnId::ALL {
            let l = self.layers[id as usize].lock().unwrap();
            let name = id.name();
            match &l.grant {
                None => out.push(format!("{name}: no layer")),
                Some(a) => {
                    let shown = l.parts.iter().filter(|p| p.shown).count();
                    let bytes: u64 = l.parts.iter().filter(|p| p.shown).map(|p| p.rect.area() * 2).sum();
                    let port = a.out.port.map_or("no port".to_string(), |p| format!("SCL {}/{}/{}", p.dev, p.chn, p.port));
                    out.push(format!("{name}: {port} {}x{}+{}+{} — {shown} windows shown of {} regions, {} KiB fetched per frame", a.out.area.w, a.out.area.h, a.out.area.x, a.out.area.y, l.parts.len(), bytes / 1024));
                    for p in &l.parts {
                        let own = p.own.map(|o| format!(" own {o:016x}")).unwrap_or_default();
                        let alpha = p.alpha.map(|a| format!(" alpha {a}")).unwrap_or_default() + if p.layer > 0 { " floating" } else { "" };
                        out.push(format!("  {} {} {}x{}+{}+{} items {}{own}{alpha} moves {} fades {} redraws {}", p.handle, if p.shown { "shown " } else { "hidden" }, p.rect.w, p.rect.h, p.rect.x, p.rect.y, p.content.len(), p.moves, p.fades, p.redraws));
                    }
                    for c in &l.covers {
                        match c.shows {
                            Some((r, [cr, cg, cb])) => out.push(format!("  {} cover  {}x{}+{}+{} #{cr:02x}{cg:02x}{cb:02x} changes {}", c.handle, r.w, r.h, r.x, r.y, c.changes)),
                            None => out.push(format!("  {} cover  hidden changes {}", c.handle, c.changes)),
                        }
                    }
                }
            }
        }
        out
    }

    /// The scene shown now and its revision, to be drawn into a photo as it is.
    pub fn scene_now(&self) -> Option<(u64, Arc<Scene>)> {
        self.state.lock().unwrap().scene.clone()
    }

    /// The media layer shows the photo's scene until [`Self::photo_done`]; once its canvases are handed over the
    /// pipeline hears from which of the port's frames on they are in ([`Msg::PhotoUi`]).
    pub fn photo(&self, job: PhotoJob) {
        self.change(|s| {
            s.photo = Some(job);
            true
        });
    }

    /// The photo is taken (or given up): the media layer follows the scene again.
    pub fn photo_done(&self) {
        self.change(|s| s.photo.take().is_some());
    }

    /// The video needs memory for something above the menu in the precedence and microcamd's share has shrunk :
    /// the thread gives back what no output shows, then, while still over the share, what `g` names — the menu's
    /// pictures (each told failed for lack of resources), the scene off the outputs (the menu hears `not-shown` while it
    /// does not fit).
    pub fn give_back(&self, g: GiveBack) {
        self.change(|s| {
            s.give_back = Some(g);
            true
        });
    }

    /// Whether `id`'s finished base needs a round to be shown: its layer draws the items it was baked for, or a photo
    /// waits for its set.
    fn drawn_after(&self, id: RgnId) -> bool {
        let s = self.state.lock().unwrap();
        s.rgn[id as usize].as_ref().is_some_and(|(_, o)| o.port.is_some()) || (id == RgnId::Media && s.photo.is_some())
    }

    /// microcamd's share moved: the menu is told what it is now and what microcamd holds of it.
    pub fn share_moved(&self) {
        self.followers.to_menu(&Event::Account { menu_bytes: mi::layer::share(), held: mi::layer::held() });
    }

    /// The pipeline refused the frames' layout this thread sent, and why: the menu is told once, for the revision the
    /// layout was laid out for.
    pub fn frames_refused(&self, r: Refusal) {
        self.change(|s| {
            s.refused = Some(r);
            true
        });
    }
}

/// What the `ui` gives back of microcamd's share when the precedence takes it for the video ([`Ui::give_back`]), and
/// what took it, for the menu.
#[derive(Clone, Debug, Default)]
pub struct GiveBack {
    pub pictures: bool,
    pub scene: bool,
    pub holders: Vec<Holder>,
}

pub fn spawn(pipeline: Sender<Msg>, patches: [Arc<PatchBox>; 3]) -> Result<Arc<Ui>, String> {
    let engine = match mi::gfx::open() {
        Ok(()) => true,
        Err(e) => {
            log::error!("ui: the 2D engine did not open ({e}); the ui shows on no output");
            false
        }
    };
    let ui = Arc::new_cyclic(|me: &std::sync::Weak<Ui>| {
        let me = me.clone();
        let res = Resources::spawn(move |c| {
            if let Some(ui) = me.upgrade() {
                ui.res_changed(c);
            }
        });
        Ui { state: Mutex::new(State::default()), wake: Condvar::new(), layers: Default::default(), pipeline: Mutex::new(pipeline), res, followers: Arc::default(), video: Mutex::default() }
    });
    // a canvas with a newer patch set handed on: its revision may be presented now
    for b in &patches {
        let me = Arc::downgrade(&ui);
        b.on_handed(Box::new(move || {
            if let Some(ui) = me.upgrade() {
                ui.notice();
            }
        }));
    }
    let shared = ui.clone();
    let outs = patches.map(Out::new);
    thread::Builder::new().name("ui".into()).spawn(move || run(shared, engine, outs)).map_err(|e| e.to_string())?;
    Ok(ui)
}

/// An RGN output's layer: the regions over the clusters of its items.
#[derive(Default)]
struct Layer {
    /// the port and frame the pipeline allows it on
    grant: Option<Attached>,
    parts: Vec<Part>,
    covers: Vec<Cover>,
    /// handles whose destroy failed: they may still be on a port, so they are tried again and never reused
    stuck: Vec<u32>,
    /// drawing rounds so far
    round: u64,
    /// a region the last round could not make for lack of microcamd's share, and why: the menu hears `not-shown`
    short: Option<String>,
}

/// One region: a hardware window over one cluster of the output's items, or hidden with its canvas kept for the
/// cluster's return; and the composer that keeps the canvas the driver lends.
struct Part {
    handle: u32,
    /// in the output frame
    rect: Rect,
    composer: Composer,
    attached: bool,
    shown: bool,
    /// what the canvas holds: the items by key and place relative to `rect` (equal content: the same pixels)
    content: Vec<(u64, i64, i64)>,
    /// the own-window group it serves
    own: Option<u64>,
    /// the window's constant alpha (its one tile's fade, the canvas holding the tile whole); None per-pixel alpha
    alpha: Option<u8>,
    /// 1 for an own-window group's floating window (over the others), else 0
    layer: u32,
    /// canvases the driver keeps for it (3 for a floating window, updated as often as it moves)
    canvases: u32,
    /// those canvases out of microcamd's share (ARGB4444, 2 B a pixel each), released with the part
    _charge: Charge,
    /// the round it was hidden in: the longest hidden gives way first
    hidden_at: u64,
    moves: u64,
    fades: u64,
    redraws: u64,
}

/// An RGN window's `n` ARGB4444 canvases of `size`.
fn rgn_bytes(size: Size, n: u64) -> u64 {
    size.w as u64 * size.h as u64 * 2 * n
}

/// One cover: a solid rectangle the port's cover engine lays under the windows in place of a drawn one.
struct Cover {
    handle: u32,
    attached: bool,
    /// what it shows: its rectangle in the port's frame and colour; None hidden
    shows: Option<(Rect, [u8; 3])>,
    /// the item it shows, by key
    key: u64,
    changes: u64,
}

/// A cover an item asks for: its rectangle in the port's frame, colour and key.
#[derive(Clone, Copy)]
struct CoverWant {
    rect: Rect,
    rgb: [u8; 3],
    key: u64,
}

impl Cover {
    fn show(&mut self, port: ChnPort, frame: Size, w: CoverWant) -> mi::Result<()> {
        if self.attached {
            mi::rgn::display_cover(self.handle, port, true, frame, w.rect, w.rgb)?;
        } else {
            mi::rgn::attach_cover(self.handle, port, frame, w.rect, w.rgb)?;
            self.attached = true;
        }
        self.shows = Some((w.rect, w.rgb));
        self.key = w.key;
        self.changes += 1;
        Ok(())
    }
}

impl Layer {
    /// Every region destroyed (the driver frees a canvas once no frame reads it); a failure is kept to try again.
    fn take_off(&mut self, id: RgnId) {
        let mut left = Vec::new();
        // the parts' charges go after their regions are destroyed: what the book lets go is in the driver's hands then
        let parts = std::mem::take(&mut self.parts);
        let covers = self.covers.drain(..).map(|c| c.handle);
        for h in parts.iter().map(|p| p.handle).chain(covers).chain(std::mem::take(&mut self.stuck)) {
            if let Err(e) = mi::rgn::destroy(h) {
                log::warn!("ui: {} region {h}: {e}", id.name());
                left.push(h);
            }
        }
        drop(parts);
        self.stuck = left;
    }

    /// Every region not shown destroyed: its canvases back to microcamd's share.
    fn drop_hidden(&mut self, id: RgnId) {
        let (hidden, kept): (Vec<Part>, Vec<Part>) = std::mem::take(&mut self.parts).into_iter().partition(|p| !p.shown);
        self.parts = kept;
        for p in hidden {
            if let Err(e) = mi::rgn::destroy(p.handle) {
                log::warn!("ui: {} region {}: {e}", id.name(), p.handle);
                self.stuck.push(p.handle);
            }
        }
    }

    fn free_handle(&self, id: RgnId) -> Option<u32> {
        let base = HANDLE_BASE[id as usize];
        (base..base + HANDLES).find(|h| !self.parts.iter().any(|p| p.handle == *h) && !self.stuck.contains(h))
    }

    /// A new cover for the output, not yet on its port; None when the port has all it may.
    fn new_cover(&mut self, id: RgnId) -> Option<usize> {
        let base = COVER_BASE[id as usize];
        let h = (base..base + COVERS as u32).find(|h| !self.covers.iter().any(|c| c.handle == *h) && !self.stuck.contains(h))?;
        if let Err(e) = mi::rgn::create_cover(h) {
            log::error!("ui: {} cover {h}: {e}", id.name());
            return None;
        }
        self.covers.push(Cover { handle: h, attached: false, shows: None, key: 0, changes: 0 });
        Some(self.covers.len() - 1)
    }

    /// The longest hidden region not in `taken` destroyed (its handle and memory free again); false when there is none.
    /// `plan` holds part indices and is fixed up.
    fn evict(&mut self, id: RgnId, taken: &mut Vec<bool>, plan: &mut [Option<usize>]) -> bool {
        let Some(v) = (0..self.parts.len()).filter(|&k| !taken[k] && !self.parts[k].shown).min_by_key(|&k| self.parts[k].hidden_at) else {
            return false;
        };
        let p = self.parts.remove(v);
        taken.remove(v);
        for k in plan.iter_mut().flatten() {
            if *k > v {
                *k -= 1;
            }
        }
        if let Err(e) = mi::rgn::destroy(p.handle) {
            log::warn!("ui: {} region {}: {e}", id.name(), p.handle);
            self.stuck.push(p.handle);
        }
        true
    }
}

/// An RGN output's renderer.
struct RgnDraw {
    id: RgnId,
}

/// What a round does with an output's bake.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    /// none: it shows another output's frames, or has no layout
    Idle,
    /// the one it shows: not built just now, or its frame not whole
    Keep,
    /// the bake of its still frame, by its key
    Want(u64),
}

fn run(ui: Arc<Ui>, engine: bool, mut outs: [Out; 3]) {
    let mut fonts = Fonts::new();
    let mut tiles = Tiles::new();
    let rgns = RgnId::ALL.map(|id| RgnDraw { id });
    let mut scratch: Option<Scratch> = None;
    let mut seen = 0;
    let mut retry: Option<Duration> = None;
    // what each output last told the menu it presents: the scene's owner (its generation: a new `hello` on the same
    // connection starts again), the output as built, the revision
    let mut told: [Option<(u64, u64, u64, bool)>; 3] = [None; 3];
    let mut drew: Option<Instant> = None;
    // the revision the menu was told cannot be shown, by the scene's owner; and per output, for its own layer
    let mut unshown: Option<(u64, u64)> = None;
    let mut unshown_out: [Option<(u64, u64)>; 3] = [None; 3];
    // a bake's job short of microcamd's share, between rounds: its outputs hear `not-shown` in the next
    let mut bake_short: Option<(u64, String)> = None;
    // the photo the pipeline was last told of, and the media output as built then (0: told it goes without its menu)
    let mut photo_told: Option<(u64, u64)> = None;
    // what the pipeline was last given of the outputs' frames, and the scene's revision they were laid out for
    let mut frames_told = Frames::default();
    let mut frames_rev = 0;
    // the USB output as last built: while it is being built anew (a stream start) its layout follows the scene for it
    let mut usb_last: Option<OutputUi> = None;
    // a job finished since the last round and something waits for a round: the next one runs whatever the state
    let mut force = false;
    // the still frames' bakes, by key: each output names the one it shows
    let mut bakes: HashMap<u64, Bake> = HashMap::new();
    let mut noticed = 0;
    loop {
        let jobs = bakes.values().any(|b| b.job.is_some());
        let head = {
            let mut s = ui.state.lock().unwrap();
            let mut due = loop {
                if s.version != seen || s.notices != noticed || force {
                    break true;
                }
                // with jobs running nothing is waited for: they go on a band at a time
                if jobs {
                    break false;
                }
                let Some(r) = retry else {
                    s = ui.wake.wait(s).unwrap();
                    continue;
                };
                let (g, t) = ui.wake.wait_timeout(s, r).unwrap();
                s = g;
                if t.timed_out() {
                    break true;
                }
            };
            // at most one drawing per frame of the fastest overlay output: what comes meanwhile is drawn together, so
            // `presented` paces the menu at the frame rate; the jobs use the time left
            let frame = s.rgn.iter().flatten().map(|(_, o)| o.fps).max().filter(|f| *f > 0).map(|f| Duration::from_secs(1) / f);
            let change = s.version != seen;
            if let (true, true, Some(frame), Some(t)) = (due, change, frame, drew) {
                let left = frame.saturating_sub(t.elapsed());
                if !left.is_zero() {
                    if jobs {
                        due = false;
                    } else {
                        drop(s);
                        thread::sleep(left);
                        s = ui.state.lock().unwrap();
                    }
                }
            }
            due.then(|| {
                if change {
                    drew = Some(Instant::now());
                }
                seen = s.version;
                noticed = s.notices;
                (s.scene.as_ref().map(|(_, sc)| sc.clone()), s.scene.as_ref().map_or(0, |(r, _)| *r), ui.followers.menu_at(), (s.rgn.clone(), s.unplaced.clone()), s.photo.as_ref().map(|j| (j.id, j.scene.clone())), s.refused.take(), std::mem::take(&mut s.give_back))
            })
        };
        let Some((scene, revision, menu, (rgn_outs, unplaced), photo, refused, give_back)) = head else {
            // one band of every running job: each bake's base. A finished one makes its set, which every output showing
            // it puts and holds its revision by, and the canvas handed with it tells `presented`; a round follows only
            // for what needs one — a newer base to begin, a layer to draw it into, a photo waiting for it
            if let Some(sc) = scratch.as_mut() {
                for (key, b) in bakes.iter_mut().filter(|(_, b)| b.job.is_some()) {
                    match b.step(&tiles, sc) {
                        Ok(None) => {}
                        Ok(Some(again)) => {
                            force |= again;
                            for id in RgnId::ALL {
                                if outs[id as usize].bake == Some(*key) {
                                    outs[id as usize].show(b);
                                    force |= ui.drawn_after(id);
                                }
                            }
                        }
                        Err(e) => {
                            log::error!("ui: the menu over the pictures not baked: {e}");
                            b.job = None;
                            force = true;
                            if e.code == Code::Resources {
                                bake_short = Some((*key, e.error));
                            }
                        }
                    }
                }
            }
            continue;
        };
        force = false;
        if let Some(g) = give_back {
            let shown: HashSet<u64> = RgnId::ALL.iter().flat_map(|id| ui.layer_keys(*id)).collect();
            tiles.trim(&shown);
            for id in RgnId::ALL {
                ui.layers[id as usize].lock().unwrap().drop_hidden(id);
            }
            // the spare frames go first, then the pictures: their windows are black in the frames made from here on,
            // the frames before them let go as these are handed
            if g.pictures && !mi::layer::room(0) {
                log::warn!("ui: the menu's pictures not shown: the video needs microcamd's share ({} KiB held of {} KiB)", mi::layer::held() >> 10, mi::layer::share() >> 10);
                ui.res.give_way(g.holders.clone());
            }
            if g.scene && !mi::layer::room(0) {
                log::warn!("ui: the scene goes off the outputs: the video needs microcamd's share ({} KiB held of {} KiB)", mi::layer::held() >> 10, mi::layer::share() >> 10);
                for id in RgnId::ALL {
                    ui.layers[id as usize].lock().unwrap().take_off(id);
                }
                for o in outs.iter_mut() {
                    o.idle();
                }
                bakes.clear();
                tiles.trim(&HashSet::new());
                scratch = None;
            }
        }
        // the layout last sent did not fit the media heap: the menu hears it once, for the revision it was laid out for
        if let (Some(r), Some((m, owner))) = (refused, menu.as_ref()) {
            if scene.is_some() && unshown != Some((*owner, frames_rev)) {
                unshown = Some((*owner, frames_rev));
                for output in RgnId::ALL.into_iter().filter(|id| rgn_outs[*id as usize].is_some()).map(RgnId::output) {
                    m.send(&Event::NotShown { output, revision: frames_rev, code: r.code, reason: r.error.clone(), holders: r.holders.clone() });
                }
            }
        }
        // an output whose layer cannot be put on its port shows none of the scene: the menu hears it once a revision
        let mut not_shown = |id: RgnId, code: Code, reason: &str, holders: &dyn Fn() -> Vec<Holder>| {
            let Some((m, owner)) = menu.as_ref().filter(|_| scene.is_some()) else { return };
            if unshown_out[id as usize] != Some((*owner, revision)) {
                unshown_out[id as usize] = Some((*owner, revision));
                m.send(&Event::NotShown { output: id.output(), revision, code, reason: reason.to_string(), holders: holders() });
            }
        };
        for id in RgnId::ALL {
            if let Some(why) = &unplaced[id as usize] {
                not_shown(id, Code::Refused, why, &Vec::new);
            }
        }
        if let Some((key, why)) = bake_short.take() {
            for id in RgnId::ALL.into_iter().filter(|id| outs[*id as usize].bake == Some(key)) {
                not_shown(id, Code::Resources, &why, &|| ui.holders());
            }
        }
        // an output showing the revision drawn: the menu hears it once per revision and output as built
        let mut presented = |slot: usize, output: Output, built: u64, revision: u64, complete: bool| {
            let Some((m, owner)) = &menu else { return };
            // once per revision as built, never an older one after a newer
            if scene.is_none() || told[slot].is_some_and(|(o, b, r, d)| (o, b) == (*owner, built) && (r, d) >= (revision, complete)) {
                return;
            }
            told[slot] = Some((*owner, built, revision, complete));
            m.send(&Event::Presented { output, revision, complete });
        };
        // the photo hears once per media output as built where its menu is, or that it goes without one
        let mut tell_photo = |generation: u64, from: Option<u64>, seq: u64| {
            let Some((id, _)) = &photo else { return };
            if photo_told == Some((*id, generation)) {
                return;
            }
            photo_told = Some((*id, generation));
            let _ = ui.pipeline.lock().unwrap().send(Msg::PhotoUi { id: *id, generation, from, seq });
        };
        retry = None;
        // each output's plane — what the scene puts into its frame (its full-colour pictures) and the pictures whose
        // size is still coming — for the output as built; an absent USB output for its last frame; the media output
        // with a photo's scene while it is taken
        if let Some((_, o)) = rgn_outs[RgnId::Usb as usize].as_ref() {
            usb_last = Some(o.ui);
        }
        let media_scene = photo.as_ref().map(|(_, s)| &**s).or(scene.as_deref());
        let outs_ui: [Option<OutputUi>; 3] = RgnId::ALL.map(|id| match id {
            RgnId::Usb => usb_last,
            _ => rgn_outs[id as usize].as_ref().map(|(_, o)| o.ui),
        });
        let planes: [Option<frame::Plane>; 3] = RgnId::ALL.map(|id| {
            let s = if id == RgnId::Media { media_scene } else { scene.as_deref() }?;
            let o = outs_ui[id as usize].as_ref()?;
            Some(frame::plane(s, o.raster, o.level, o.paused.is_some(), &|id| ui.res.size(id)))
        });
        // each picture's largest window shown, and its largest prepared ahead when larger (in no frame): its form is made
        // for what is shown first, then the one ahead into microcamd's spare
        let largest = largest_of(planes.iter().flatten().flat_map(|p| &p.pictures));
        let ahead = largest_of(planes.iter().flatten().flat_map(|p| &p.prepared));
        let names: HashSet<&str> = largest.keys().chain(ahead.keys()).copied().collect();
        let keys: HashMap<&str, PictureId> = names
            .iter()
            .filter_map(|id| {
                let shown = largest.get(id).map(|p| (p.crop, p.window.size()));
                let more = ahead.get(id).filter(|a| largest.get(id).is_none_or(|s| a.window.area() > s.window.area())).map(|p| (p.crop, p.window.size()));
                Some((*id, ui.res.frame(id, shown, more)?))
            })
            .collect();
        let framed: [Option<Framed>; 3] = planes.each_ref().map(|p| p.as_ref().map(|p| framed(p, &|id| keys.get(id).copied(), &|id| ui.res.failed(id), &|id| ui.res.coming(id), &|id| ui.res.short(id))));
        // the layout each output's frame takes once whole; until then, and for an output not built just now (HDMI being
        // built anew), the one told: nothing changes. The media output exists only for a recording or a photo: its
        // layout goes with it
        let told_layout = |id: RgnId| match id {
            RgnId::Usb => frames_told.usb.clone(),
            RgnId::Media => frames_told.media.clone(),
            RgnId::Hdmi => frames_told.hdmi.clone(),
        };
        let layout_now: [Option<Layout>; 3] = RgnId::ALL.map(|id| match (&outs_ui[id as usize], &framed[id as usize]) {
            (None, _) if id == RgnId::Media => None,
            (None, _) => told_layout(id),
            (Some(_), None) => None,
            (Some(_), Some(Framed { whole: Some((l, _)), .. })) => Some(l.clone()),
            (Some(_), Some(_)) => told_layout(id),
        });
        let frames_of = |l: &[Option<Layout>; 3]| Frames {
            usb: l[RgnId::Usb as usize].clone(),
            media: l[RgnId::Media as usize].clone(),
            hdmi: l[RgnId::Hdmi as usize].clone(),
            // the outputs' levels say what each shows only while there is a scene (the policy compares frames by it)
            scene: scene.is_some(),
        };
        // only a changed layout goes to the pipeline; one it refuses it keeps and builds when it can
        let tell_frames = |frames: Frames, told: &mut Frames, rev: &mut u64| {
            if frames != *told {
                let _ = ui.pipeline.lock().unwrap().send(Msg::Frames(frames.clone()));
                *told = frames;
                *rev = revision;
            }
        };
        if !engine || mi::gfx::lost() {
            tell_frames(frames_of(&layout_now), &mut frames_told, &mut frames_rev);
            tell_photo(0, None, 0);
            continue;
        }
        // an output's items, and whether its images are all drawn at the scale shown (a JPEG decoded smaller is asked
        // again); the resources each form was used for
        let pictures = |id: &str| ui.res.picture(id);
        let rgba_used = std::cell::RefCell::new(HashSet::<String>::new());
        let place = |scene: Option<&Scene>, out: &OutputUi, fonts: &mut Fonts, holes: &[frame::Picture]| -> (Vec<Item>, bool) {
            let Some(s) = scene else {
                return (Vec::new(), true);
            };
            let (items, uses) = shown(s, out, fonts, &pictures, holes);
            let complete = uses.iter().fold(true, |all, u| {
                rgba_used.borrow_mut().insert(u.res.clone());
                let enough = u.scale > 0.0 && ui.res.want(&u.res, u.scale);
                u.drawn && enough && all
            });
            (items, complete)
        };
        // every output's items first: their tiles are admitted together, keeping what every layer shows now
        let mut rgn_done = [true; 3];
        // the media layer shows a photo's scene while it is taken
        let rgn_scene = |id: RgnId| match (&photo, id) {
            (Some((_, s)), RgnId::Media) => Some(&**s),
            _ => scene.as_deref(),
        };
        // an output reading another output's frames (joined) has no items of its own: that output's layer and patches
        // are its — (the output, its generation, the pool it reads)
        let joined: Vec<(RgnId, u64, u32)> = RgnId::ALL.into_iter().filter_map(|id| rgn_outs[id as usize].as_ref().and_then(|(g, o)| o.joined.map(|p| (id, *g, p)))).collect();
        // the items of every output built with a layer or patches of its own whose frame is whole; an output whose frame
        // is not whole changes nothing
        let rgn_items: Vec<Option<Vec<Item>>> = RgnId::ALL
            .into_iter()
            .zip(&mut rgn_done)
            .map(|(id, done)| {
                let (_, o) = rgn_outs[id as usize].as_ref().filter(|(_, o)| o.joined.is_none())?;
                let f = framed[id as usize].as_ref();
                if f.is_some_and(|f| f.whole.is_none()) {
                    return None;
                }
                let (i, d) = place(rgn_scene(id), &o.ui, &mut fonts, f.map_or(&[][..], |f| &f.list));
                // a picture waiting for room: its window black meanwhile
                *done = d && !f.is_some_and(|f| f.short);
                Some(i)
            })
            .collect();
        let mut keep: HashSet<u64> = HashSet::new();
        for id in RgnId::ALL {
            keep.extend(ui.layer_keys(id));
        }
        // what the running jobs draw from, and the bases shown while one runs
        let job_keys: Vec<u64> = bakes.values().flat_map(|b| b.keys().into_iter().chain(b.base_items.iter().map(|i| i.key))).collect();
        keep.extend(job_keys.iter().copied());
        // one scratch, as wide as the widest layer
        let widest = rgn_outs.iter().flatten().map(|(_, o)| o.ui.raster.w).max().unwrap_or(0);
        if widest > 0 && scratch.as_ref().is_none_or(|s| s.width() < widest) {
            scratch = None;
            match Scratch::new(widest) {
                Ok(s) => scratch = Some(s),
                Err(e) => {
                    log::error!("ui: no scratch {widest} wide: {e}");
                    retry = Some(RETRY);
                    if short_code(&e) == Code::Resources {
                        for id in RgnId::ALL.into_iter().filter(|id| rgn_outs[*id as usize].is_some()) {
                            not_shown(id, Code::Resources, &format!("the menu's drawing band: {e}"), &|| ui.holders());
                        }
                    }
                }
            }
        }
        // a bake takes every item (a cover lies only on the live window's port)
        let baked = rgn_items.iter().flatten().flatten();
        let all: Vec<Item> = baked.chain(bakes.values().flat_map(|b| &b.base_items)).cloned().collect();
        let scene_keys: Vec<u64> = rgn_items.iter().flatten().flatten().map(|i| i.key).collect();
        let t0 = Instant::now();
        match tiles.admit(&all, &keep) {
            Ok(px) if px > 0 => log::debug!("ui: {} kpx of tiles drawn in {} ms, {} KiB held ({} KiB of microcamd's share held)", px / 1000, t0.elapsed().as_millis(), tiles.bytes() / 1024, mi::layer::held() / 1024),
            Ok(_) => {}
            Err(e) => {
                // the outputs keep what they show and try again (a photo or a freeze may be holding the heap); a
                // photo goes without its menu
                log::error!("ui: the scene is not shown: {e}");
                retry = Some(RETRY);
                if let Some((m, owner)) = menu.as_ref().filter(|(_, o)| scene.is_some() && unshown != Some((*o, revision))) {
                    unshown = Some((*owner, revision));
                    let code = short_code(&e);
                    let holders = if code == Code::Resources { ui.holders() } else { Vec::new() };
                    let outputs = RgnId::ALL.into_iter().filter(|id| rgn_outs[*id as usize].is_some()).map(RgnId::output);
                    for output in outputs {
                        m.send(&Event::NotShown { output, revision, code, reason: e.to_string(), holders: holders.clone() });
                    }
                }
                tell_photo(0, None, 0);
                continue;
            }
        }
        // no scratch: nothing is baked — with no output built its layouts go as laid out, else they wait for one
        let Some(sc) = scratch.as_mut() else {
            if widest == 0 {
                tell_frames(frames_of(&layout_now), &mut frames_told, &mut frames_rev);
            }
            continue;
        };
        // what each output does with the bakes this round
        let role: [Role; 3] = RgnId::ALL.map(|id| {
            let k = id as usize;
            if joined.iter().any(|(j, ..)| *j == id) || layout_now[k].is_none() {
                return Role::Idle;
            }
            match (&rgn_outs[k], &rgn_items[k], &framed[k]) {
                (Some((_, o)), Some(items), Some(Framed { whole: Some((layout, pics)), .. })) => Role::Want(Bake::key(layout, pics, o.ui.paused.is_some(), items)),
                _ => Role::Keep,
            }
        });
        for (o, p) in outs.iter_mut().zip(&role) {
            if *p == Role::Idle {
                o.idle();
            }
        }
        // each still frame wanted baked once, for every output showing it: a bake made this round or kept from the last
        // is taken as it is; else the bake one of those outputs showed, when no output shows or wants it as it is, is
        // carried on to the new frame (its patches kept where they hold); else a new one
        let wanted: HashSet<u64> = role.iter().filter_map(|p| if let Role::Want(k) = p { Some(*k) } else { None }).collect();
        let kept: HashSet<u64> = role.iter().zip(&outs).filter(|(p, _)| **p == Role::Keep).filter_map(|(_, o)| o.bake).collect();
        let mut baked_seq = [None; 3];
        let mut made: Vec<u64> = Vec::new();
        for id in RgnId::ALL {
            let Role::Want(key) = role[id as usize] else { continue };
            if made.contains(&key) {
                continue;
            }
            made.push(key);
            let ids: Vec<RgnId> = RgnId::ALL.into_iter().filter(|i| role[*i as usize] == Role::Want(key)).collect();
            if !bakes.contains_key(&key) {
                let was = ids.iter().filter_map(|i| outs[*i as usize].bake).find(|b| bakes.contains_key(b) && !wanted.contains(b) && !kept.contains(b));
                let b = was.and_then(|b| bakes.remove(&b)).unwrap_or_default();
                bakes.insert(key, b);
            }
            let bake = bakes.get_mut(&key).expect("inserted");
            let (Some((_, o)), Some(items), Some(Framed { whole: Some((layout, pics)), .. })) = (&rgn_outs[id as usize], &rgn_items[id as usize], &framed[id as usize]) else { continue };
            let base = bake.head(&ui, layout, pics, o.ui.paused.is_some(), items, &tiles, sc);
            if let Err(e) = &base {
                // the set stays as it was: a new geometry keeps its output's frame
                log::error!("ui: {}: the menu over the pictures not baked: {e}", id.name());
                retry = Some(retry.map_or(RETRY, |r| r.min(RETRY)));
                if e.code == Code::Resources {
                    for &i in &ids {
                        not_shown(i, Code::Resources, &e.error, &|| ui.holders());
                    }
                }
            }
            for i in ids {
                let out = &mut outs[i as usize];
                out.bake = Some(key);
                out.wanted = base.as_ref().ok().map(|b| (*b, revision, rgn_done[i as usize]));
                baked_seq[i as usize] = out.show(bake);
            }
        }
        bakes.retain(|k, _| outs.iter().any(|o| o.bake == Some(*k)));
        // the layouts go: an output baking its own set once its box holds the set made for the layout; one reading
        // another's frames with that output; one not built just now as it is laid out
        let gate = |id: RgnId| -> Option<Layout> {
            let k = id as usize;
            let new = layout_now[k].as_ref().filter(|l| Some(*l) != told_layout(id).as_ref());
            let Some(l) = new else { return layout_now[k].clone() };
            let goes = match (role[k], &rgn_outs[k]) {
                (Role::Want(_), Some((_, o))) => outs[k].made_for(l, o.ui.paused.is_some()),
                _ => true,
            };
            if goes { Some(l.clone()) } else { told_layout(id) }
        };
        let mut gated = RgnId::ALL.map(gate);
        for (id, _, pool) in &joined {
            let lead = RgnId::ALL.into_iter().find(|l| *l as u32 == *pool);
            if lead.is_some_and(|l| gated[l as usize] != layout_now[l as usize]) {
                gated[*id as usize] = told_layout(*id);
            }
        }
        tell_frames(frames_of(&gated), &mut frames_told, &mut frames_rev);
        // each RGN output's items as it shows them: the solid ones its covers show, the rest its windows draw
        let rgn_split: Vec<Option<(Vec<Item>, Vec<Item>)>> = RgnId::ALL
            .into_iter()
            .map(|id| {
                let k = id as usize;
                let ((_, o), list) = (rgn_outs[k].as_ref()?, rgn_items[k].as_ref()?);
                // what the output shows: while its base is being baked, the base's items and the newest own groups; of
                // them only the ones over the built frame's visible live part go into the layer (holes and items over
                // pictures are in the patches, where the holes keep the z order over the pictures)
                let view = outs[k].bake.and_then(|b| bakes.get(&b)).map_or_else(|| list.clone(), |b| b.view(list));
                let list = on_live(&view, &o.visible);
                let c = covers(&list, o.area, COVERS);
                let (mut solid, mut drawn) = (Vec::new(), Vec::new());
                // an item at no opacity shows nothing: no window
                for (i, it) in list.iter().enumerate() {
                    if c.contains(&i) {
                        solid.push(it.clone())
                    } else if it.fade > 0.0 {
                        drawn.push(it.clone())
                    }
                }
                Some((solid, drawn))
            })
            .collect();
        // the outputs whose layer shows what was drawn, as built; what they present is their newest set handed on
        let mut shows: Vec<(RgnId, u64)> = Vec::new();
        for (r, (o, split)) in rgns.iter().zip(rgn_outs.iter().zip(rgn_split)) {
            match (o, split) {
                // an output whose box holds the set of a geometry not built yet: its layer stays as it is until it is
                (Some((_, out)), Some(_)) if !outs[r.id as usize].on_built(out) => {}
                (Some((generation, out)), Some((solid, drawn))) => {
                    // no menu in the photo, only the holes of its pictures (none at its level: no layer either): any frame
                    // from now on
                    let bare = r.id == RgnId::Media && photo.is_some() && solid.is_empty() && drawn.iter().all(|i| i.hole);
                    let seq = baked_seq[RgnId::Media as usize];
                    if bare && seq.is_some() {
                        tell_photo(*generation, None, seq.unwrap_or(0));
                    }
                    // nothing of the scene at its level and nothing on its port (no layer put there): it shows the
                    // revision as its frames do
                    let nothing = solid.is_empty() && drawn.is_empty() && !(r.id == RgnId::Media && photo.is_some());
                    match r.draw(&ui, *generation, out, &solid, drawn, &tiles, sc) {
                        Ok(true) if bare => {}
                        // the photo's canvases are handed over: every frame the port programs from the next on has them
                        Ok(true) if r.id == RgnId::Media && photo.is_some() => match out.port.map_or(Err(std::io::Error::other("no port")), mi::rgn::frames) {
                            Ok((programmed, _)) if seq.is_some() => tell_photo(*generation, Some(programmed + 1), seq.unwrap_or(0)),
                            Ok(_) => {}
                            // the photo cannot be told which frame holds its menu: it fails when it waits too long
                            Err(e) => log::error!("ui: media: the photo's frame: {e}"),
                        },
                        Ok(true) => shows.push((r.id, *generation)),
                        Ok(_) if nothing && ui.layer_bare(r.id) => shows.push((r.id, *generation)),
                        Ok(_) => {}
                        Err(again) => {
                            retry = Some(retry.map_or(again, |r| r.min(again)));
                            let short = ui.layers[r.id as usize].lock().unwrap().short.take();
                            if let Some(why) = short {
                                not_shown(r.id, Code::Resources, &why, &|| ui.holders());
                            }
                        }
                    }
                    // covered: no layer, the whole menu is in the patches
                    match (out.port, seq) {
                        (None, Some(seq)) if r.id == RgnId::Media && photo.is_some() => tell_photo(*generation, None, seq),
                        (None, _) => shows.push((r.id, *generation)),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        // an output reading another's frames presents with it: the same canvases go to both
        for (id, generation) in shows {
            let Some((rev, complete)) = outs[id as usize].shown() else { continue };
            presented(id as usize, id.output(), generation, rev, complete);
            for (j, g, _) in joined.iter().filter(|(_, _, p)| *p == id as u32) {
                presented(*j as usize, j.output(), *g, rev, complete);
            }
        }
        // tiles neither shown nor in the scene go at once: the media heap is short
        let mut held: HashSet<u64> = scene_keys.into_iter().collect();
        // what the bakes draw from now: a bake no output shows holds none any more
        held.extend(bakes.values().flat_map(|b| b.keys().into_iter().chain(b.base_items.iter().map(|i| i.key))));
        for id in RgnId::ALL {
            held.extend(ui.layer_keys(id));
        }
        tiles.trim(&held);
        // released resources the scene names no more go, and the forms nothing showed
        let rgba_used = rgba_used.into_inner();
        let rgba: HashSet<&str> = rgba_used.iter().map(String::as_str).collect();
        let nv12: HashSet<&str> = names.iter().copied().collect();
        ui.res.sweep(&scene.as_deref().map(|s| s.resources()).unwrap_or_default(), &rgba, &nv12);
        // the bands go with what needs them: the bake's with the last bake, the scratch once no layer shows anything
        // and nothing is to be drawn; the next drawing makes them again
        if bakes.is_empty() {
            let bare = RgnId::ALL.into_iter().all(|id| ui.layer_bare(id)) && rgn_items.iter().flatten().all(Vec::is_empty);
            if bare {
                scratch = None;
            } else if let Some(s) = scratch.as_mut() {
                s.unbake();
            }
        }
    }
}

impl RgnDraw {
    /// Bring the output's layer to `solid` and `items`: a cover per solid item; a region per
    /// cluster of the others — the one already holding the same pixels moved or shown again as it is, one of the same
    /// size redrawn where it differs, or a new one; regions no cluster needs are hidden with their canvas kept. A cover
    /// comes before the windows lose its item and goes after they have it again, so no frame shows a hole. True when the
    /// layer shows them, false when it is not on its port now; when to try again after a failure.
    fn draw(&self, ui: &Ui, generation: u64, out: &RgnOutput, solid: &[Item], items: Vec<Item>, tiles: &Tiles, scratch: &mut Scratch) -> Result<bool, Duration> {
        let name = self.id.name();
        // drawn only while the pipeline allows the layer on its port
        let mut l = ui.layers[self.id as usize].lock().unwrap();
        if !l.grant.as_ref().is_some_and(|a| a.generation == generation && a.out.same_layer(out)) {
            return Ok(false);
        }
        if !l.stuck.is_empty() {
            let stuck = std::mem::take(&mut l.stuck);
            l.stuck = stuck.into_iter().filter(|h| mi::rgn::destroy(*h).is_err()).collect();
        }
        let Some(port) = out.port else { return Ok(false) };
        let clip = out.area;
        let at = |r: Rect| (r.x - clip.x, r.y - clip.y);
        l.round += 1;
        let round = l.round;
        let frame = clip.size();
        let cover_want: Vec<CoverWant> = solid
            .iter()
            .map(|i| {
                let r = i.rect.intersect(clip);
                CoverWant { rect: Rect { x: r.x - clip.x, y: r.y - clip.y, ..r }, rgb: i.solid.unwrap_or_default(), key: i.key }
            })
            .collect();
        // covers already showing what is wanted stay; the others wanted come now — on the cover of the same item (it
        // moved), a hidden one, one whose item is gone, a new one; a cover whose item the windows draw again changes
        // only after they have it
        let drawn: HashSet<u64> = items.iter().map(|i| i.key).collect();
        let mut placed: Vec<bool> = cover_want.iter().map(|_| false).collect();
        let mut kept = vec![false; l.covers.len()];
        for (k, w) in cover_want.iter().enumerate() {
            if let Some(c) = (0..l.covers.len()).find(|&c| !kept[c] && l.covers[c].shows == Some((w.rect, w.rgb))) {
                kept[c] = true;
                placed[k] = true;
            }
        }
        for (k, w) in cover_want.iter().enumerate() {
            if placed[k] {
                continue;
            }
            let free = |c: &Cover| c.shows.is_none() || !drawn.contains(&c.key);
            let pick = (0..l.covers.len())
                .find(|&c| !kept[c] && l.covers[c].shows.is_some() && l.covers[c].key == w.key && free(&l.covers[c]))
                .or_else(|| (0..l.covers.len()).find(|&c| !kept[c] && l.covers[c].shows.is_none()))
                .or_else(|| (0..l.covers.len()).find(|&c| !kept[c] && free(&l.covers[c])));
            let c = match pick {
                Some(c) => c,
                None => match l.new_cover(self.id) {
                    Some(c) => {
                        kept.push(false);
                        c
                    }
                    None => continue,
                },
            };
            if let Err(e) = l.covers[c].show(port, frame, *w) {
                log::error!("ui: {name}: cover {}: {e}", l.covers[c].handle);
                return Err(RETRY);
            }
            kept[c] = true;
            placed[k] = true;
        }
        // each cluster's items, bottom to top, their content relative to the cluster's rectangle, and the window's
        // constant alpha: a window holding one tile opaque all over takes the tile's fade, its canvas the tile whole
        let want: Vec<(Rect, Vec<Item>, Vec<(u64, i64, i64)>, Option<u64>, Option<u8>)> = clusters(&items, clip, GWINS)
            .into_iter()
            .map(|c| {
                let mut list: Vec<Item> = c.items.iter().map(|&i| items[i].clone()).collect();
                let alpha = match list.as_mut_slice() {
                    [one] if one.opaque && one.fade < 1.0 => {
                        let a = (one.fade * 255.0).round() as u8;
                        one.fade = 1.0;
                        Some(a)
                    }
                    _ => None,
                };
                let content = list.iter().map(|i| (i.key, i.rect.x as i64 - c.rect.x as i64, i.rect.y as i64 - c.rect.y as i64)).collect();
                (c.rect, list, content, c.own, alpha)
            })
            .collect();
        // a region for every cluster: the one holding the same pixels first (its window moves or comes back as it is),
        // else one of the same size (redrawn where it differs), else a new one
        let mut taken = vec![false; l.parts.len()];
        let mut plan: Vec<Option<usize>> = vec![None; want.len()];
        for pass in 0..2 {
            for (k, w) in want.iter().enumerate() {
                if plan[k].is_some() {
                    continue;
                }
                let fits = |p: &Part| if pass == 0 { p.content == w.2 } else { p.rect.size() == w.0.size() };
                if let Some(pi) = (0..l.parts.len()).find(|&pi| !taken[pi] && fits(&l.parts[pi])) {
                    taken[pi] = true;
                    plan[k] = Some(pi);
                }
            }
        }
        // what nothing needs now goes off its window, its canvas kept
        for (pi, p) in l.parts.iter_mut().enumerate() {
            if !taken[pi] && p.shown {
                let (x, y) = at(p.rect);
                if let Err(e) = mi::rgn::display(p.handle, port, false, x, y, p.alpha, p.layer) {
                    log::error!("ui: {name}: region {} hide: {e}", p.handle);
                    return Err(RETRY);
                }
                p.shown = false;
                p.hidden_at = round;
            }
        }
        let mut st = Stats::default();
        let (mut made, mut moved, mut faded, mut redrawn) = (0, 0, 0, 0);
        let t0 = Instant::now();
        for k in 0..want.len() {
            let (rect, list, content, own, alpha) = &want[k];
            let pi = match plan[k] {
                Some(pi) => pi,
                None => loop {
                    // handles and memory: the longest hidden regions give way
                    let h = match l.free_handle(self.id) {
                        Some(h) => h,
                        None if l.evict(self.id, &mut taken, &mut plan) => continue,
                        None => {
                            log::error!("ui: {name}: no region handle left");
                            return Err(RETRY);
                        }
                    };
                    // its two canvases are charged before mi_rgn makes them: a window that does not fit is not made
                    let charge = match Charge::take(rgn_bytes(rect.size(), 2), Use::Rgn) {
                        Ok(c) => c,
                        Err(_) if l.evict(self.id, &mut taken, &mut plan) => continue,
                        Err(e) => {
                            let by: Vec<String> = mi::layer::usage().iter().map(|(u, b, _)| format!("{} {}", u.name(), b >> 10)).collect();
                            log::error!("ui: {name}: region {}: {e}; held, KiB: {}", rect.size(), by.join(", "));
                            l.short = Some(format!("a {} window of the menu: {e}", rect.size()));
                            return Err(RETRY);
                        }
                    };
                    match mi::rgn::create(h, rect.size()) {
                        Ok(()) => {}
                        Err(e) if l.evict(self.id, &mut taken, &mut plan) => {
                            log::info!("ui: {name}: region {h} {}: {e}; a hidden region freed", rect.size());
                            continue;
                        }
                        Err(e) => {
                            log::error!("ui: {name}: region {h} {}: {e}", rect.size());
                            return Err(RETRY);
                        }
                    }
                    l.parts.push(Part { handle: h, rect: *rect, composer: Composer::new(), attached: false, shown: false, content: Vec::new(), own: *own, alpha: None, layer: 0, canvases: 2, _charge: charge, hidden_at: 0, moves: 0, fades: 0, redraws: 0 });
                    taken.push(true);
                    made += 1;
                    break l.parts.len() - 1;
                },
            };
            let p = &mut l.parts[pi];
            let was = p.rect;
            if was != *rect {
                p.composer.moved(rect.x as i64 - was.x as i64, rect.y as i64 - was.y as i64);
                p.rect = *rect;
            }
            p.own = *own;
            let layer = own.is_some() as u32;
            if layer == 1 && p.canvases < 3 {
                match Charge::take(rgn_bytes(rect.size(), 3), Use::Rgn) {
                    Ok(c) => match mi::rgn::canvases(p.handle, 3) {
                        Ok(()) => {
                            p.canvases = 3;
                            p._charge = c;
                        }
                        Err(e) => log::warn!("ui: {name}: region {} third canvas: {e}", p.handle),
                    },
                    Err(e) => log::warn!("ui: {name}: region {} third canvas: {e}", p.handle),
                }
            }
            let (x, y) = at(*rect);
            let change = p.attached && (!p.shown || was != *rect || p.alpha != *alpha || p.layer != layer);
            let mut changed = false;
            if p.composer.needs(list, *rect) {
                // a canvas lent and not given back (a failure below) comes again as it is, and is redrawn whole
                let (phy, stride) = match mi::rgn::canvas(p.handle) {
                    Ok(c) => c,
                    Err(e) => {
                        log::error!("ui: {name}: region {} canvas: {e}", p.handle);
                        return Err(RETRY);
                    }
                };
                let canvas = Surface { phy, fmt: Fmt::Argb4444, w: rect.w, h: rect.h, stride };
                match p.composer.show(scratch, &mut Sink::Engine(canvas), *rect, list.clone(), tiles) {
                    Ok(s) => {
                        st.areas += s.areas;
                        st.bands += s.bands;
                        st.blits += s.blits;
                        st.pixels += s.pixels;
                    }
                    Err(e) => {
                        p.content.clear();
                        log::error!("ui: {name}: {e}");
                        return Err(RETRY);
                    }
                }
                // the window's new place and look set while the canvas is lent land with the new pixels, in one frame
                if change {
                    if let Err(e) = mi::rgn::display(p.handle, port, true, x, y, *alpha, layer) {
                        p.content.clear();
                        log::error!("ui: {name}: region {} show at {x},{y}: {e}", p.handle);
                        return Err(RETRY);
                    }
                    changed = true;
                }
                if let Err(e) = mi::rgn::update(p.handle) {
                    p.content.clear();
                    log::error!("ui: {name}: region {}: {e}", p.handle);
                    return Err(RETRY);
                }
                p.content = content.clone();
                p.redraws += 1;
                redrawn += 1;
            }
            if !p.attached {
                if let Err(e) = mi::rgn::attach(p.handle, port, x, y, *alpha, layer) {
                    log::error!("ui: {name}: region {} on SCL {}/{}/{}: {e}", p.handle, port.dev, port.chn, port.port);
                    return Err(RETRY);
                }
                p.attached = true;
                p.shown = true;
                p.alpha = *alpha;
                p.layer = layer;
            } else if change {
                if !changed {
                    if let Err(e) = mi::rgn::display(p.handle, port, true, x, y, *alpha, layer) {
                        log::error!("ui: {name}: region {} show at {x},{y}: {e}", p.handle);
                        return Err(RETRY);
                    }
                }
                if p.shown && was != *rect {
                    p.moves += 1;
                    moved += 1;
                }
                if p.shown && p.alpha != *alpha {
                    p.fades += 1;
                    faded += 1;
                }
                p.shown = true;
                p.alpha = *alpha;
                p.layer = layer;
            }
        }
        // the windows hold everything else now: the covers left over take what is still wanted, the rest go
        let mut left = cover_want.iter().enumerate().filter(|(k, _)| !placed[*k]).map(|(_, w)| *w);
        for c in 0..l.covers.len() {
            let cv = &mut l.covers[c];
            let Some((r, rgb)) = cv.shows.filter(|_| !kept[c]) else {
                continue;
            };
            let r = match left.next() {
                Some(w) => cv.show(port, frame, w),
                None => {
                    mi::rgn::display_cover(cv.handle, port, false, frame, r, rgb).map(|()| {
                        cv.shows = None;
                        cv.changes += 1;
                    })
                }
            };
            if let Err(e) = r {
                log::error!("ui: {name}: cover {}: {e}", cv.handle);
                return Err(RETRY);
            }
        }
        if st.pixels > 0 || made > 0 || moved > 0 || faded > 0 {
            let hidden = l.parts.iter().filter(|p| !p.shown).count();
            let covers = l.covers.iter().filter(|c| c.shows.is_some()).count();
            log::debug!("ui: {name} {}: {} windows ({made} new, {moved} moved, {faded} faded, {redrawn} redrawn, {hidden} hidden), {covers} covers, {} areas, {} bands, {} blits, {} kpx in {} ms", out.area.size(), want.len(), st.areas, st.bands, st.blits, st.pixels / 1000, t0.elapsed().as_millis());
        }
        Ok(true)
    }
}

/// Each picture's largest window among `pics`, by its resource.
fn largest_of<'a>(pics: impl Iterator<Item = &'a frame::Picture>) -> std::collections::HashMap<&'a str, &'a frame::Picture> {
    let mut m: std::collections::HashMap<&str, &frame::Picture> = std::collections::HashMap::new();
    for p in pics {
        let e = m.entry(p.res.as_str()).or_insert(p);
        if p.window.area() > e.window.area() {
            *e = p;
        }
    }
    m
}

/// A picture in an output's frame as the `ui` lays it: its decode, the part of the source picture shown, its window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Pic {
    id: PictureId,
    crop: Rect,
    window: Rect,
}

/// An output's frame as the scene lays it: the pictures in it — the plane's, less those whose resource failed: its
/// windows, its holes and its still set all come from this one list — and, once it is whole (no picture's size and no
/// picture's form still coming), its layout and its pictures as laid. A picture whose form waits for room in
/// microcamd's share does not hold the frame: the room it waits for is held by the frame the output shows, given back
/// only when the output leaves it. The frame goes without it — its window black, the output not complete — and the
/// picture comes in once its room is back, as another picture in a window that stays.
struct Framed {
    list: Vec<frame::Picture>,
    whole: Option<(Layout, Vec<Pic>)>,
    /// a picture of the frame waits for room: the output is not complete
    short: bool,
}

/// `p` framed: `key` gives a picture's NV12 form held, `failed` and `coming` say of a resource whether it failed and
/// whether what it lacks may still come, `short` whether its form waits for room.
fn framed(p: &frame::Plane, key: &dyn Fn(&str) -> Option<PictureId>, failed: &dyn Fn(&str) -> bool, coming: &dyn Fn(&str) -> bool, short: &dyn Fn(&str) -> bool) -> Framed {
    let list: Vec<frame::Picture> = p.pictures.iter().filter(|q| !failed(&q.res)).cloned().collect();
    let waits: Vec<bool> = list.iter().map(|q| key(&q.res).is_none() && short(&q.res)).collect();
    let sized = !p.sizing.iter().any(|id| coming(id));
    let pics: Option<Vec<Pic>> = sized
        .then(|| list.iter().zip(&waits).filter(|(_, w)| !**w).map(|(q, _)| key(&q.res).map(|id| Pic { id, crop: q.crop, window: q.window })).collect())
        .flatten();
    let whole = pics.map(|pics| (Layout { raster: p.raster, live: p.live, windows: list.iter().map(|q| q.window).collect() }, pics));
    Framed { list, whole, short: waits.contains(&true) }
}

/// `pics` scaled to their windows (the cache the compositors share).
fn cleans(ui: &Ui, pics: &[Pic]) -> Result<Vec<Clean>, Unbaked> {
    pics.iter()
        .map(|p| {
            let frame = ui.res.decoded(p.id).ok_or_else(|| Unbaked { code: Code::Refused, error: format!("picture {} is not held", p.id.0) })?;
            mi::compose::clean(p.id, &frame, p.crop, p.window, mi::scl::User::Ui).map_err(|e| unbaked(&e, format!("picture {}", p.id.0)))
        })
        .collect()
}

/// Why a still set could not be made: `resources` when microcamd's share had no room for it, else `refused`.
#[derive(Debug)]
struct Unbaked {
    code: Code,
    error: String,
}

impl std::fmt::Display for Unbaked {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&self.error)
    }
}

/// `e` met making `what`, typed for the menu.
fn unbaked(e: &mi::layer::LayerError, what: impl std::fmt::Display) -> Unbaked {
    Unbaked { code: short_code(e), error: format!("{what}: {e}") }
}

/// `resources` when `e` is microcamd's share short, else `refused`.
fn short_code(e: &mi::layer::LayerError) -> Code {
    match e {
        mi::layer::LayerError::Budget { .. } => Code::Resources,
        _ => Code::Refused,
    }
}

/// The visible live part of the frame laid out by `tag`, as the compositor has it (none when covered).
fn tag_visible(tag: &Tag) -> Vec<Rect> {
    frame::assemble(tag.raster, tag.live, &tag.windows).visible
}

/// The still region of the frame laid out by `tag`: outside the visible live part, as the compositor has it.
fn tag_still(tag: &Tag) -> Vec<Rect> {
    frame::region(&[Rect::full(tag.raster)], &tag_visible(tag), &tag.windows)
}

/// One still frame's bake — the pictures of a frame and the menu over them and the black between them, outside the
/// visible live part, baked into patches — shared by every output showing that still frame ([`Out`]). The patches are
/// the **base** — every item but the own-window groups' — and over it the **own patches**, one per own group over the
/// still content, baked from everything there. A cursor moved bakes only its own patches; a change of the rest, other
/// pictures in the same geometry included, is baked by a [`BaseJob`] a band at a time between the `ui`'s rounds, the
/// set keeping the old pictures and base meanwhile, the own groups floating over it as their GOP windows do. A new
/// geometry is baked whole at once.
#[derive(Default)]
struct Bake {
    /// the geometry of the set, its pictures as laid and scaled to their windows (the compositor takes the same ones)
    tag: Option<Tag>,
    pics: Vec<Pic>,
    stills: Vec<Clean>,
    /// the base: its patches, the items they were baked from and those items' key ([`base_key`])
    base: Vec<Patch>,
    base_items: Vec<Item>,
    base_key: Option<u64>,
    /// the own patches, and the items of the latest round they are baked from
    own: Vec<Patch>,
    items: Vec<Item>,
    /// the latest round's base key: what a job finished for another key leaves to bake again
    wanted: Option<u64>,
    job: Option<BaseJob>,
    /// the set made of the above, one `Arc` put into the box of every output showing it: its geometry, pictures and
    /// each patch's place and revision
    set: Option<(Tag, Vec<Pic>, Vec<(Rect, u64)>, Arc<StillSet>)>,
}

/// A base being baked a band at a time: the items (no own group's) and their key, the pictures under them when they
/// change (as laid, and scaled), the areas, the patches done.
struct BaseJob {
    key: u64,
    items: Vec<Item>,
    pics: Option<(Vec<Pic>, Vec<Clean>)>,
    areas: Vec<Rect>,
    done: Vec<Patch>,
    /// the area being baked, and its revision
    cur: Option<(mi::layer::BakeJob, u64)>,
    t0: Instant,
    px: u64,
}

/// The key of a base: the geometry, the pictures and every item's tile, place, fade and hole.
fn base_key(tag: &Tag, pics: &[Pic], items: &[Item]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    tag.hash(&mut h);
    pics.hash(&mut h);
    for i in items {
        (i.key, i.rect, i.fade.to_bits(), i.hole).hash(&mut h);
    }
    h.finish()
}

/// A patch's revision: what changes whenever its pixels would — the geometry, its place, the pictures and the items
/// over it.
fn patch_rev(tag: &Tag, pics: &[Pic], rect: Rect, items: &[Item]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    tag.hash(&mut h);
    rect.hash(&mut h);
    for p in pics.iter().filter(|p| !p.window.intersect(rect).is_empty()) {
        p.hash(&mut h);
    }
    for i in items.iter().filter(|i| !i.rect.intersect(rect).is_empty()) {
        (i.key, i.rect, i.fade.to_bits(), i.hole).hash(&mut h);
    }
    h.finish()
}

impl Bake {
    /// What decides a bake: the output's geometry (the live window gone when `paused`), its pictures and every item as
    /// drawn. Outputs with equal ones show the same still frame and take one bake.
    fn key(layout: &Layout, pics: &[Pic], paused: bool, items: &[Item]) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (layout.raster, if paused { Rect::default() } else { layout.live }, &layout.windows).hash(&mut h);
        pics.hash(&mut h);
        for i in items {
            (i.key, i.rect, i.fade.to_bits(), i.hole, i.own).hash(&mut h);
        }
        h.finish()
    }

    /// What an output shows of `items` (bottom to top): all of them, but while a base is being baked the base's items
    /// with `items`' own groups floating on top.
    fn view(&self, items: &[Item]) -> Vec<Item> {
        if self.job.is_none() {
            return items.to_vec();
        }
        self.base_items.iter().cloned().chain(items.iter().filter(|i| i.own.is_some()).cloned()).collect()
    }

    /// The round's part: `items` (bottom to top, its tiles in `tiles`) over the pictures `pics` in the geometry `layout`
    /// — all of the frame still when the output's live picture is `paused`. A new geometry is baked whole now; a new
    /// base or new pictures whose patches the set already has are taken at once, any other is begun as a job (one
    /// running finishes first); the own patches are baked now over what the set has. The key of the base `items` want
    /// (the set holds them whole once its base is that); or why not (the set stays as it was).
    #[allow(clippy::too_many_arguments)]
    fn head(&mut self, ui: &Ui, layout: &Layout, pics: &[Pic], paused: bool, items: &[Item], tiles: &Tiles, scratch: &mut Scratch) -> Result<u64, Unbaked> {
        let live = if paused { Rect::default() } else { layout.live };
        let tag = Tag { raster: layout.raster, live, windows: layout.windows.clone() };
        let others: Vec<Item> = items.iter().filter(|i| i.own.is_none()).cloned().collect();
        let key = base_key(&tag, pics, &others);
        self.items = items.to_vec();
        self.wanted = Some(key);
        if self.tag.as_ref() != Some(&tag) {
            // a new geometry: its set made whole before the layout goes to the pipeline
            let t0 = Instant::now();
            let stills = cleans(ui, pics)?;
            let views: Vec<mi::layer::Still> = stills.iter().map(|c| c.still()).collect();
            let mut base = Vec::new();
            let mut px = 0;
            for rect in bake_areas(&others, &tag_still(&tag)) {
                let rev = patch_rev(&tag, pics, rect, &others);
                let frame = mi::layer::bake(scratch, rect, &others, tiles, &views).map_err(|e| unbaked(&e, format_args!("patch {rect:?}")))?;
                px += rect.area();
                base.push(Patch { rect, rev, frame: Arc::new(frame) });
            }
            log::debug!("ui: base of {} patches over a new layout {}x{}, {} kpx baked in {} ms", base.len(), tag.raster.w, tag.raster.h, px / 1000, t0.elapsed().as_millis());
            self.job = None;
            self.stills = stills;
            self.pics = pics.to_vec();
            self.base = base;
            self.base_items = others;
            self.base_key = Some(key);
            self.own.clear();
            self.tag = Some(tag.clone());
        } else if self.base_key != Some(key) && self.job.is_none() {
            // other pictures in the same geometry: made now, laid with the base baked over them
            let new = if self.pics.as_slice() != pics { Some((pics.to_vec(), cleans(ui, pics)?)) } else { None };
            let areas = bake_areas(&others, &tag_still(&tag));
            // every area the base would be made of already in the set (always so when there is none: a frame all live
            // picture) — taken at once, so this round draws and presents the change; else baked by a job
            let kept: Option<Vec<Patch>> = areas
                .iter()
                .map(|&rect| {
                    let rev = patch_rev(&tag, pics, rect, &others);
                    self.base.iter().find(|p| p.rect == rect && p.rev == rev).cloned()
                })
                .collect();
            match kept {
                Some(base) => {
                    if let Some((p, c)) = new {
                        self.pics = p;
                        self.stills = c;
                    }
                    self.base = base;
                    self.base_items = others;
                    self.base_key = Some(key);
                }
                None => self.job = Some(BaseJob { key, items: others, pics: new, areas, done: Vec::new(), cur: None, t0: Instant::now(), px: 0 }),
            }
        }
        self.bake_own(tiles, scratch)?;
        self.make_set();
        Ok(key)
    }

    /// The own patches over the set, from what an output shows of the latest round's items.
    fn bake_own(&mut self, tiles: &Tiles, scratch: &mut Scratch) -> Result<(), Unbaked> {
        let Some(tag) = self.tag.clone() else { return Ok(()) };
        let view = self.view(&self.items);
        let own_items: Vec<Item> = self.items.iter().filter(|i| i.own.is_some()).cloned().collect();
        let views: Vec<mi::layer::Still> = self.stills.iter().map(|c| c.still()).collect();
        let mut own = Vec::new();
        let mut px = 0;
        let t0 = Instant::now();
        for rect in bake_areas(&own_items, &tag_still(&tag)) {
            let rev = patch_rev(&tag, &self.pics, rect, &view);
            if let Some(p) = self.own.iter().find(|p| p.rect == rect && p.rev == rev) {
                own.push(p.clone());
                continue;
            }
            let frame = mi::layer::bake(scratch, rect, &view, tiles, &views).map_err(|e| unbaked(&e, format_args!("own patch {rect:?}")))?;
            px += rect.area();
            own.push(Patch { rect, rev, frame: Arc::new(frame) });
        }
        if px > 0 {
            log::debug!("ui: {} own patches, {} kpx baked in {:.1} ms", own.len(), px / 1000, t0.elapsed().as_secs_f64() * 1e3);
        }
        self.own = own;
        Ok(())
    }

    /// The pictures, the base and the own patches as a new set when they differ from the last one made.
    fn make_set(&mut self) {
        let Some(tag) = &self.tag else { return };
        let patches: Vec<Patch> = self.base.iter().chain(&self.own).cloned().collect();
        let revs: Vec<(Rect, u64)> = patches.iter().map(|p| (p.rect, p.rev)).collect();
        if self.set.as_ref().is_some_and(|(t, p, r, _)| t == tag && *p == self.pics && *r == revs) {
            return;
        }
        log::debug!("ui: a set of {} pictures, {} patches ({} own)", self.stills.len(), patches.len(), self.own.len());
        let set = Arc::new(StillSet { pictures: self.stills.clone(), patches });
        self.set = Some((tag.clone(), self.pics.clone(), revs, set));
    }

    /// The tiles the running job and the own patches draw from.
    fn keys(&self) -> Vec<u64> {
        let own = self.items.iter().filter(|i| i.own.is_some()).map(|i| i.key);
        let Some(j) = &self.job else { return own.collect() };
        j.items.iter().map(|i| i.key).chain(j.cur.iter().flat_map(|(b, _)| b.keys())).chain(own).collect()
    }

    /// One band of the running base job; once it is done: its pictures and base taken, the own patches baked over them,
    /// the set made — Some(whether another base waits for a round), else None.
    fn step(&mut self, tiles: &Tiles, scratch: &mut Scratch) -> Result<Option<bool>, Unbaked> {
        let (Some(j), Some(tag)) = (self.job.as_mut(), self.tag.as_ref()) else {
            self.job = None;
            return Ok(Some(true));
        };
        let (pics, stills) = match &j.pics {
            Some((p, c)) => (p, c),
            None => (&self.pics, &self.stills),
        };
        let views: Vec<mi::layer::Still> = stills.iter().map(|c| c.still()).collect();
        loop {
            if let Some((b, _)) = j.cur.as_mut() {
                let whole = b.step(scratch, tiles, &views).map_err(|e| unbaked(&e, format_args!("patch {:?}", b.rect())))?;
                if !whole {
                    return Ok(None);
                }
                let (b, rev) = j.cur.take().expect("baking");
                j.px += b.rect().area();
                j.done.push(Patch { rect: b.rect(), rev, frame: Arc::new(b.patch()) });
                continue;
            }
            // the next area: kept when the base in the set has it, else begun
            let Some(rect) = (j.done.len() < j.areas.len()).then(|| j.areas[j.done.len()]) else { break };
            let rev = patch_rev(tag, pics, rect, &j.items);
            if let Some(p) = self.base.iter().find(|p| p.rect == rect && p.rev == rev) {
                j.done.push(p.clone());
                continue;
            }
            j.cur = Some((mi::layer::BakeJob::begin(scratch, rect, &j.items).map_err(|e| unbaked(&e, format_args!("patch {rect:?}")))?, rev));
        }
        let j = self.job.take().expect("running");
        log::debug!("ui: base of {} patches, {} kpx baked in {} ms (by bands)", j.done.len(), j.px / 1000, j.t0.elapsed().as_millis());
        if let Some((p, c)) = j.pics {
            self.pics = p;
            self.stills = c;
        }
        self.base = j.done;
        self.base_items = j.items;
        self.base_key = Some(j.key);
        self.bake_own(tiles, scratch)?;
        self.make_set();
        Ok(Some(self.wanted != Some(j.key)))
    }
}

/// An output's side of the still sets: its compositor's box, the sets put there numbered (each output its own
/// numbers, so its `presented` follows its own canvases), and the bake it shows.
struct Out {
    out: Arc<PatchBox>,
    /// the last set's number
    seq: u64,
    /// the set last put: its geometry, pictures and each patch's place and revision
    put: Option<(Tag, Vec<Pic>, Vec<(Rect, u64)>)>,
    /// the sets put and not yet known handed: each one's number, the scene's revision it holds and whether complete
    sets: VecDeque<(u64, u64, bool)>,
    /// the bake it shows, by its key in the round's bakes
    bake: Option<u64>,
    /// the base it wants of that bake, for the scene's revision and whether complete there
    wanted: Option<(u64, u64, bool)>,
}

impl Out {
    fn new(out: Arc<PatchBox>) -> Self {
        Self { out, seq: 0, put: None, sets: VecDeque::new(), bake: None, wanted: None }
    }

    /// `b`'s set into the box when it differs from the last one put; the set's number when it holds the base wanted
    /// (the revision it is wanted for whole), None while that base is still being baked.
    fn show(&mut self, b: &Bake) -> Option<u64> {
        if let Some((tag, pics, revs, set)) = &b.set {
            if !self.put.as_ref().is_some_and(|(t, p, r)| t == tag && p == pics && r == revs) {
                self.seq += 1;
                self.out.put(self.seq, tag.clone(), set.clone());
                self.put = Some((tag.clone(), pics.clone(), revs.clone()));
            }
        }
        let (key, revision, complete) = self.wanted?;
        if b.base_key != Some(key) || self.put.is_none() {
            return None;
        }
        match self.sets.back_mut() {
            Some(last) if last.0 == self.seq => *last = (self.seq, revision, complete),
            _ => self.sets.push_back((self.seq, revision, complete)),
        }
        Some(self.seq)
    }

    /// The newest revision whose set is handed on, and whether it was complete; the older sets are dropped.
    fn shown(&mut self) -> Option<(u64, bool)> {
        let handed = self.out.handed();
        let n = self.sets.iter().take_while(|s| s.0 <= handed).count();
        self.sets.drain(..n.checked_sub(1)?);
        self.sets.front().map(|s| (s.1, s.2))
    }

    /// Nothing shown nor kept (the output shows another's frames, or no scene): the box empty; the set numbers go on,
    /// so a set made later is newer than any handed before.
    fn idle(&mut self) {
        if self.put.is_none() && self.bake.is_none() {
            return;
        }
        log::debug!("ui: the still set of an output with nothing of its own to show dropped");
        self.put = None;
        self.sets.clear();
        self.bake = None;
        self.wanted = None;
        self.out.clear();
    }

    /// Whether the set in the box is made for `layout`'s geometry (the live window gone when `paused`).
    fn made_for(&self, layout: &Layout, paused: bool) -> bool {
        self.put.as_ref().is_some_and(|(t, ..)| t.raster == layout.raster && t.windows == layout.windows && t.live == if paused { Rect::default() } else { layout.live })
    }

    /// Whether the set in the box is for the geometry `o` is built with — compared by the visible live part, the
    /// layer's cut. A set of the live picture alone fits any: the policy fits that live window to the panel itself.
    fn on_built(&self, o: &RgnOutput) -> bool {
        match &self.put {
            None => true,
            Some((t, ..)) if t.windows.is_empty() && t.live == Rect::full(t.raster) => true,
            Some((t, ..)) => t.raster == o.ui.raster && tag_visible(t) == o.visible,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Another picture in a window that stays: the same geometry, so no new layout; only the patches over that window
    /// get a new revision (baked again over the new picture), the others are kept, and the base's key moves.
    #[test]
    fn another_picture_in_the_same_window_renews_only_the_patches_over_it() {
        let r = |x, y, w, h| Rect { x, y, w, h };
        let photo = r(0, 0, 3840, 1800);
        let thumb = r(0, 1800, 768, 360);
        let tag = Tag { raster: Size::new(3840, 2160), live: Rect::default(), windows: vec![photo, thumb] };
        let pic = |id, window: Rect| Pic { id: PictureId(id), crop: Rect::full(window.size()), window };
        let (a, b) = (vec![pic(1, photo), pic(2, thumb)], vec![pic(3, photo), pic(2, thumb)]);
        let (over_photo, over_thumb) = (r(100, 100, 400, 80), r(100, 1900, 200, 100));
        assert_ne!(patch_rev(&tag, &a, over_photo, &[]), patch_rev(&tag, &b, over_photo, &[]));
        assert_eq!(patch_rev(&tag, &a, over_thumb, &[]), patch_rev(&tag, &b, over_thumb, &[]));
        assert_ne!(base_key(&tag, &a, &[]), base_key(&tag, &b, &[]));
    }

    /// The gallery entered: the photo over the whole frame and five thumbnails in a strip. While a size or a form is
    /// still coming the frame is not whole — no layout, so no `Frames` — and once every picture is decoded it is one
    /// layout with every window, bottom to top.
    #[test]
    fn a_photo_and_five_thumbnails_coming_give_no_layout_until_all_are_decoded() {
        let uhd = Size::new(3840, 2160);
        let pic = |k: u32, window: Rect| frame::Picture { node: k as u64, res: format!("p{k}"), window, crop: Rect::full(window.size()) };
        let strip = (1..=5).map(|k| pic(k, Rect { x: 128 + (k - 1) * 736, y: 1780, w: 640, h: 360 }));
        let mut plane = frame::Plane { raster: uhd, live: Rect::full(uhd), pictures: std::iter::once(pic(0, Rect::full(uhd))).chain(strip).collect(), prepared: Vec::new(), sizing: vec!["p6".into()] };
        let decoded = std::cell::RefCell::new(HashMap::<String, PictureId>::new());
        let key = |id: &str| decoded.borrow().get(id).copied();
        let (no, yes) = (|_: &str| false, |_: &str| true);
        assert!(framed(&plane, &key, &no, &yes, &no).whole.is_none(), "a size coming");
        plane.sizing.clear();
        for k in 0..6 {
            assert!(framed(&plane, &key, &no, &yes, &no).whole.is_none(), "{k} of 6 decoded");
            decoded.borrow_mut().insert(format!("p{k}"), PictureId(k + 1));
        }
        let (layout, pics) = framed(&plane, &key, &no, &yes, &no).whole.expect("whole");
        assert_eq!(layout.windows, plane.pictures.iter().map(|q| q.window).collect::<Vec<_>>());
        assert_eq!((layout.raster, layout.live, pics.len()), (uhd, Rect::full(uhd), 6));
        // a size no longer coming (its resource failed) does not hold the frame
        plane.sizing.push("gone".into());
        assert!(framed(&plane, &key, &no, &no, &no).whole.is_some());
    }

    /// A picture whose resource failed is left out of the one list: no window in the layout, no hole in the menu, and
    /// the frame does not wait for it.
    #[test]
    fn a_failed_picture_is_neither_a_window_nor_a_hole() {
        let uhd = Size::new(3840, 2160);
        let pic = |k: u32, window: Rect| frame::Picture { node: k as u64, res: format!("p{k}"), window, crop: Rect::full(window.size()) };
        let (a, b) = (Rect { x: 0, y: 0, w: 1920, h: 2160 }, Rect { x: 1920, y: 0, w: 1920, h: 2160 });
        let plane = frame::Plane { raster: uhd, live: Rect::full(uhd), pictures: vec![pic(0, a), pic(1, b)], prepared: Vec::new(), sizing: Vec::new() };
        let key = |id: &str| (id == "p0").then_some(PictureId(1));
        let f = framed(&plane, &key, &|id| id == "p1", &|_| true, &|_| false);
        assert_eq!(f.list, vec![pic(0, a)], "the holes");
        let (layout, pics) = f.whole.expect("whole without it");
        assert_eq!((layout.windows, pics.len()), (vec![a], 1));
    }

    /// heavy.py swap: the output shows a 4K photo full screen, held by its still set; the next scene's photo cannot be
    /// decoded beside it and waits for room. The new frame goes without it — its window kept, black, the output not
    /// complete — so the output leaves the old frame and gives its photo's room back; once decoded it is a picture in a
    /// window that stays. A picture merely decoding still holds the frame.
    #[test]
    fn a_picture_waiting_for_room_does_not_hold_its_frame() {
        let uhd = Size::new(3840, 2160);
        let pic = |k: u32, window: Rect| frame::Picture { node: k as u64, res: format!("p{k}"), window, crop: Rect::full(window.size()) };
        let (half, thumb) = (Rect { x: 0, y: 0, w: 1920, h: 2160 }, Rect { x: 100, y: 100, w: 400, h: 300 });
        let plane = frame::Plane { raster: uhd, live: Rect { x: 1920, y: 0, w: 1920, h: 2160 }, pictures: vec![pic(0, half), pic(1, thumb)], prepared: Vec::new(), sizing: Vec::new() };
        let key = |id: &str| (id == "p1").then_some(PictureId(2));
        let (no, yes) = (|_: &str| false, |_: &str| true);
        assert!(framed(&plane, &key, &no, &yes, &no).whole.is_none(), "p0 decoding: the frame waits");
        let f = framed(&plane, &key, &no, &yes, &|id| id == "p0");
        let (layout, pics) = f.whole.expect("goes without p0");
        assert_eq!((layout.windows, pics, f.short), (vec![half, thumb], vec![Pic { id: PictureId(2), crop: Rect::full(thumb.size()), window: thumb }], true));
        assert_eq!(f.list.len(), 2, "its hole stays: the menu lies as over the picture");
        let all = |id: &str| Some(PictureId(if id == "p0" { 1 } else { 2 }));
        let f = framed(&plane, &all, &no, &yes, &|id| id == "p0");
        assert_eq!((f.whole.map(|(_, p)| p.len()), f.short), (Some(2), false), "decoded at last: whole, complete");
    }

    /// Two outputs at the same raster showing the same pictures in the same windows take one bake; a paused one, another
    /// window or another picture bake apart (items, the same on equal outputs, enter the key alike).
    #[test]
    fn outputs_showing_the_same_still_frame_share_one_bake_key() {
        let r = |x, y, w, h| Rect { x, y, w, h };
        let uhd = Size::new(3840, 2160);
        let layout = |windows: Vec<Rect>| Layout { raster: uhd, live: r(1920, 0, 1920, 2160), windows };
        let pic = |id, window: Rect| Pic { id: PictureId(id), crop: Rect::full(window.size()), window };
        let half = r(0, 0, 1920, 2160);
        let key = Bake::key(&layout(vec![half]), &[pic(1, half)], false, &[]);
        assert_eq!(key, Bake::key(&layout(vec![half]), &[pic(1, half)], false, &[]));
        assert_ne!(key, Bake::key(&layout(vec![half]), &[pic(1, half)], true, &[]), "paused");
        assert_ne!(key, Bake::key(&layout(vec![half]), &[pic(2, half)], false, &[]), "another picture");
        let quarter = r(0, 0, 1920, 1080);
        assert_ne!(key, Bake::key(&layout(vec![quarter]), &[pic(1, quarter)], false, &[]), "another window");
    }
}
