//! Gets the view onto microcamd's outputs: the pictures it names asked for and released, the tree sent whole, the
//! next commit only once the last is shown on every output with the live scene (later states merged into it), the
//! screens' steps down when an output cannot show them. Its requests carry numbers of their own, apart from the core's.

use std::collections::{BTreeMap, BTreeSet};

use microcam_core::controls::{Held, Holder};
use microcam_core::edit::Op;
use microcam_core::protocol::{Envelope, Event, Output, Request, ResState, Response, Source};
use microcam_core::scene::Node;
use microcam_draw::render::Fonts;

use crate::core::{Core, Freeze, Link, Screen};
use crate::view::{Down, Pic, View, FREEZE_DOWN};

/// The presenter's request numbers start here; the core's stay below.
pub const SEQ_BASE: u64 = 1 << 40;
/// The longest a step waits for an output to show it before the next is sent anyway: above the slowest gallery step
/// measured on two 4K outputs.
const STUCK_MS: u64 = 2000;
/// The heaviest step down the gallery has (`view`).
const MAX_DOWN: Down = 2;

/// A commit sent and not yet shown everywhere.
struct Flight {
    rev: u64,
    at: u64,
    whole: bool,
    /// the key that caused it, when one did
    cause: Option<(u64, String)>,
    /// each output's time to show it
    shown: Vec<(Output, u64)>,
    /// outputs that said they cannot show it: answered, not waited for
    refused: Vec<Output>,
}

/// What screen a step down belongs to: left, it starts from nothing again.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Heavy {
    None,
    Gallery,
    Freeze,
}

pub struct Presenter {
    owner: bool,
    /// the session's number: resource ids carry it, so a new session never meets the old one's
    gen: u64,
    seq: u64,
    rev: u64,
    flight: Option<Flight>,
    /// replies awaited: a commit's number to its revision
    commits: BTreeMap<u64, u64>,
    /// the tree last sent and the pictures it names
    last: Option<Vec<Node>>,
    last_pics: BTreeSet<String>,
    /// a tree refused: not sent again until the view changes
    refused: Option<Vec<Node>>,
    held: BTreeMap<String, Source>,
    pics: BTreeMap<String, Pic>,
    down: Down,
    heavy: Heavy,
    /// the first revision of the heavy screen, and whether HDMI or USB has shown one of its revisions whole: a freeze
    /// shown is not taken again smaller, the camera picture of the moment would replace it (the recording's output,
    /// with no layer on its port, presents every revision whole while no one sees it)
    heavy_from: u64,
    seen: bool,
    /// a key since the last commit sent: its time and name
    cause: Option<(u64, String)>,
    /// outputs that let a step pass its limit unshown: not waited for until they present again
    stuck: Vec<Output>,
    /// pictures released and not yet gone
    releasing: BTreeSet<String>,
    /// requests held until they are, with when they were held
    later: Vec<(u64, Envelope)>,
    pub fonts: Fonts,
    /// lines for the log: what went wrong, and each step's times when `steps`
    pub notes: Vec<String>,
    pub steps: bool,
}

impl Default for Presenter {
    fn default() -> Self {
        Self::new()
    }
}

impl Presenter {
    pub fn new() -> Self {
        Presenter {
            owner: false,
            gen: 0,
            seq: SEQ_BASE,
            rev: 0,
            flight: None,
            commits: BTreeMap::new(),
            last: None,
            last_pics: BTreeSet::new(),
            refused: None,
            held: BTreeMap::new(),
            pics: BTreeMap::new(),
            down: 0,
            heavy_from: 0,
            seen: false,
            heavy: Heavy::None,
            cause: None,
            stuck: Vec::new(),
            releasing: BTreeSet::new(),
            later: Vec::new(),
            fonts: Fonts::new(),
            notes: Vec::new(),
            steps: false,
        }
    }

    fn id(&self, core_id: &str) -> String {
        format!("{}:{core_id}", self.gen)
    }

    /// A resource event's id as the core names it, when it is this session's.
    pub fn core_id<'a>(&self, id: &'a str) -> Option<&'a str> {
        id.strip_prefix(&format!("{}:", self.gen))
    }

    fn send(&mut self, out: &mut Vec<Envelope>, req: Request) -> u64 {
        self.seq += 1;
        out.push(Envelope { seq: Some(self.seq), req });
        self.seq
    }

    /// A key came: the next commit is timed from it.
    pub fn key(&mut self, now: u64, name: String) {
        self.cause.get_or_insert((now, name));
    }

    /// A request held until the screen shown has let go of the pictures the state no longer names.
    pub fn later(&mut self, now: u64, e: Envelope) {
        self.later.push((now, e));
    }

    /// The key changed nothing on the screen and asked nothing: the next commit is not its.
    pub fn unkey(&mut self) {
        if self.flight.is_none() {
            self.cause = None;
        }
    }

    pub fn reply(&mut self, r: &Response) {
        let Some(rev) = r.seq.and_then(|s| self.commits.remove(&s)) else { return };
        match (r.ok, r.revision) {
            (true, Some(accepted)) => self.rev = accepted,
            _ => {
                self.rev = rev - 1;
                self.notes.push(format!("commit {rev} refused: {}", r.error.as_deref().unwrap_or("")));
                self.refused = self.last.take();
                self.flight = None;
            }
        }
    }

    /// An event for the scene; true when it was a `not-shown` met by a step down, which is the presenter's alone.
    pub fn event(&mut self, now: u64, e: &Event) -> bool {
        match e {
            Event::Presented { output, revision, complete } => {
                self.seen |= *complete && *revision >= self.heavy_from && *output != Output::Media;
                if self.stuck.contains(output) {
                    self.stuck.retain(|o| o != output);
                    self.notes.push(format!("{output:?} presents again"));
                }
                if let Some(f) = &mut self.flight {
                    if *revision >= f.rev && (*complete || !f.whole) && !f.shown.iter().any(|s| s.0 == *output) {
                        f.shown.push((*output, now - f.at));
                    }
                }
            }
            Event::NotShown { output, revision, reason, .. } => {
                self.notes.push(format!("{output:?} cannot show {revision}: {reason}"));
                let steps = match self.heavy {
                    Heavy::None => 0,
                    Heavy::Gallery => MAX_DOWN,
                    Heavy::Freeze if self.seen => 0,
                    Heavy::Freeze => FREEZE_DOWN,
                };
                if self.down < steps && self.flight.as_ref().is_some_and(|f| f.rev == *revision) {
                    self.down += 1;
                    self.notes.push(format!("a step down to {}", self.down));
                    self.flight = None;
                    return true;
                }
                if let Some(f) = self.flight.as_mut().filter(|f| f.rev == *revision) {
                    f.refused.push(*output);
                }
            }
            Event::Resource { id, state } => {
                if let Some(core_id) = self.core_id(id).map(String::from) {
                    let pic = match state {
                        ResState::Loading => Pic::Waiting,
                        ResState::Ready { w, h } => Pic::Ready(*w, *h),
                        ResState::Failed { code, .. } => Pic::Failed(*code),
                        ResState::Gone => {
                            self.releasing.remove(&core_id);
                            self.pics.remove(&core_id);
                            return false;
                        }
                    };
                    self.pics.insert(core_id, pic);
                }
            }
            _ => {}
        }
        false
    }

    /// The holders' pictures as the core names them (this session's ids without their session's number).
    pub fn core_holders(&self, holders: &mut [Holder]) {
        for h in holders {
            if let Held::Picture { id } = &mut h.what {
                if let Some(core_id) = self.core_id(id) {
                    *id = core_id.to_string();
                }
            }
        }
    }

    /// The outputs a step waits for: those that show the live scene now and are not stuck.
    fn live(&self, core: &Core) -> Vec<Output> {
        core.st.cam.outputs.iter().filter(|o| o.paused.is_none() && !self.stuck.contains(&o.name)).map(|o| o.name).collect()
    }

    /// Whether the step in flight is shown, or waited for long enough.
    fn landed(&mut self, now: u64, core: &Core) -> bool {
        let Some(f) = &self.flight else { return true };
        let live = self.live(core);
        let took = |f: &Flight, o: &Output| f.shown.iter().find(|s| s.0 == *o).map(|s| s.1);
        let answered = |f: &Flight, o: &Output| took(f, o).is_some() || f.refused.contains(o);
        let all = live.iter().all(|o| answered(f, o));
        if !all && now < f.at + STUCK_MS {
            return false;
        }
        let f = self.flight.take().expect("a flight");
        let stuck = live.iter().any(|o| !answered(&f, o));
        let mut line = format!("rev {}", f.rev);
        if let Some((at, key)) = &f.cause {
            line += &format!(" · {key} → sent +{} ms", f.at - at);
        }
        for o in &live {
            match took(&f, o) {
                Some(ms) => line += &format!(" · {o:?} +{ms} ms"),
                None if f.refused.contains(o) => line += &format!(" · {o:?} cannot show it"),
                None => {
                    line += &format!(" · {o:?} not shown in {STUCK_MS} ms, not waited for until it presents");
                    self.stuck.push(*o);
                }
            }
        }
        if let Some((at, _)) = &f.cause {
            if let Some(worst) = live.iter().filter_map(|o| took(&f, o)).max() {
                line += &format!(" · key → shown {} ms", f.at - at + worst);
            }
        }
        if self.steps || stuck {
            self.notes.push(line);
        }
        true
    }

    /// The scene brought up to the state: requests to send, and when to be asked again.
    pub fn update(&mut self, now: u64, core: &Core) -> (Vec<Envelope>, Option<u64>) {
        let mut out = Vec::new();
        let owner = core.st.link == Link::Owner;
        if owner != self.owner {
            self.owner = owner;
            // a new session: an empty scene at revision 0, none of the old pictures
            self.gen += 1;
            self.rev = 0;
            self.flight = None;
            self.commits.clear();
            self.last = None;
            self.last_pics.clear();
            self.refused = None;
            self.held.clear();
            self.pics.clear();
            self.releasing.clear();
            self.later.clear();
            self.stuck.clear();
        }
        if !owner {
            return (out, None);
        }
        let heavy = match (&core.st.screen, core.st.freeze) {
            (Screen::Gallery(_), _) => Heavy::Gallery,
            (_, Freeze::Off) => Heavy::None,
            _ => Heavy::Freeze,
        };
        if heavy != self.heavy {
            (self.heavy, self.down, self.heavy_from, self.seen) = (heavy, 0, self.rev + 1, false);
        }
        if !self.landed(now, core) {
            return (out, self.flight.as_ref().map(|f| f.at + STUCK_MS));
        }
        let wake = self.commit(now, core, &mut out);
        // the held requests once nothing released is still in memory — or once waited for long enough
        let due = self.flight.is_none() && self.releasing.is_empty();
        let (go, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.later).into_iter().partition(|(at, _)| due || now >= at + STUCK_MS);
        self.later = keep;
        for (at, e) in go {
            self.notes.push(format!("a request held for the pictures to go, sent after {} ms{}", now - at, if due { "" } else { " (waited out)" }));
            out.push(e);
        }
        let wake = [wake, self.later.first().map(|l| l.0 + STUCK_MS)].into_iter().flatten().min();
        (out, wake)
    }

    /// The tree sent when it changed; when to look again.
    fn commit(&mut self, now: u64, core: &Core, out: &mut Vec<Envelope>) -> Option<u64> {
        let pics = &self.pics;
        let pic = |id: &str| pics.get(id).copied().unwrap_or(Pic::Waiting);
        let drawn = View { core, pic: &pic, fonts: &self.fonts, down: self.down }.draw();
        if self.last.as_ref() == Some(&drawn.nodes) || self.refused.as_ref() == Some(&drawn.nodes) {
            return None;
        }
        for (id, src) in &drawn.pictures {
            if !self.held.contains_key(id) {
                self.held.insert(id.clone(), src.clone());
                let req = Request::Resource { id: self.id(id), source: src.clone() };
                self.send(out, req);
            }
        }
        let mut nodes = drawn.nodes.clone();
        rename(&mut nodes, &|id| self.id(id));
        let seq = self.send(out, Request::Commit { base: self.rev, ops: vec![Op::Replace { nodes }] });
        self.commits.insert(seq, self.rev + 1);
        self.flight = Some(Flight { rev: self.rev + 1, at: now, whole: drawn.whole, cause: self.cause.take(), shown: Vec::new(), refused: Vec::new() });
        // an optimistic revision: the next commit builds on this one, and a refusal sets it straight
        self.rev += 1;
        let wanted: BTreeSet<String> = drawn.pictures.iter().map(|p| p.0.clone()).collect();
        // a picture neither this tree nor the one before names is released (microcamd keeps it until no output shows it)
        let gone: Vec<String> = self.held.keys().filter(|id| !wanted.contains(*id) && !self.last_pics.contains(*id)).cloned().collect();
        for id in gone {
            self.held.remove(&id);
            self.pics.remove(&id);
            let req = Request::Release { id: self.id(&id) };
            self.send(out, req);
            self.releasing.insert(id);
        }
        self.last = Some(drawn.nodes);
        self.last_pics = wanted;
        self.refused = None;
        Some(now + STUCK_MS)
    }
}

/// Every image's resource id renamed.
fn rename(nodes: &mut [Node], f: &dyn Fn(&str) -> String) {
    use microcam_core::scene::Kind;
    for n in nodes {
        match &mut n.kind {
            Kind::Image(m) => m.res = f(&m.res),
            Kind::Group(g) => rename(&mut g.children, f),
            _ => {}
        }
    }
}
