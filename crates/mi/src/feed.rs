//! The feed thread: microcamd hands every encoder and DISP's input port their frames itself, nothing is bound into them
//!. Each feed takes the frames of one port (`MI_SYS_ChnOutputPortGetBufPa`) and hands them on through the compositor
//! ([`crate::compose`]): the port wrote the live picture into the canvases of the output's pool, which go whole to the
//! pool's readers, each at its own rate; when no live part shows, the port is off and the compositor hands its canvas
//! on its own clock.
//!
//! One thread for every feed, started with the first; the pipeline thread starts and stops feeds through it like any
//! other step, and `stop` returns once the feed holds nothing of the port or the encoder any more.
//!
//! An encoder that refuses every canvas for [`LOST_AFTER`] (it is gone: another process took its device down), or that the compositor finds holding every canvas, is lost — the feed's own destination or another reader of its
//! pool (USB's encoder reading HDMI's): the feed marks its channel in [`Lost`] and tells the pipeline once, which builds
//! that output anew. DISP is never lost: the compositor counts what it refuses.
//!
//! While a compositor hands DISP canvases at their lending, the thread also waits on the canvas module's
//! lendings ([`crate::canvas::Lent`]) and tells each such compositor its pool's newest.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use microcam_core::reconcile::ChnPort;
use mi_sys as ffi;

use crate::compose::Composer;
use crate::sys::chn_port;
use crate::{check, Result};

/// Told once, with the encoder's channel, when a feed's encoder is lost (DISP is never lost: a canvas it refuses is
/// counted).
pub type OnLost = Arc<dyn Fn(u32) + Send + Sync>;

/// The encoder channels found lost, shared by the feeds and the Hal: marked once by the feed whose compositor found it,
/// cleared by the Hal when the channel is fed, read or built anew.
#[derive(Clone, Default)]
pub struct Lost(Arc<Mutex<Vec<u32>>>);

impl Lost {
    /// `chn` marked; true when it was not.
    fn mark(&self, chn: u32) -> bool {
        let mut l = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if l.contains(&chn) {
            return false;
        }
        l.push(chn);
        true
    }

    pub fn is(&self, chn: u32) -> bool {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).contains(&chn)
    }

    pub fn clear(&self, chn: u32) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).retain(|c| *c != chn);
    }
}

/// Every frame refused for this long: the encoder is lost.
pub const LOST_AFTER: Duration = Duration::from_secs(1);

/// How long a feed may take to stop: one poll round and a compositor's give-back.
const STOP_WAIT: Duration = Duration::from_secs(2);
const POLL_MS: i32 = 50;

enum Cmd {
    Start(Feed),
    Stop(ChnPort, Sender<std::result::Result<(), String>>),
    Snap(ChnPort, crate::compose::Snap),
    Early(ChnPort, bool),
}

#[derive(Default)]
pub struct Feeder {
    cmds: Option<Sender<Cmd>>,
    thread: Option<JoinHandle<()>>,
}

impl Feeder {
    /// Hand the frames of `src` to `dst` (an encoder's input or DISP's input port: the readers of the compositor's pool,
    /// each at its rate). The port must be enabled unless covered. An encoder among the readers found lost is
    /// marked in `lost` (and `on_lost` told) once.
    pub fn start(&mut self, src: ChnPort, dst: ChnPort, way: Composer, lost: Lost, on_lost: OnLost) -> Result<()> {
        let mut port = chn_port(src);
        let mut fd = -1;
        // covered: the port is off, the compositor keeps its own clock
        if way.due().is_none() {
            check("MI_SYS_GetFd", unsafe { ffi::MI_SYS_GetFd(&mut port, &mut fd) })?;
        }
        let feed = Feed { dst, port, fd, way, handed: 0, lost, on_lost };
        if self.cmds.is_none() {
            let (tx, rx) = mpsc::channel();
            self.thread = Some(std::thread::Builder::new().name("feed".into()).spawn(move || run(rx)).expect("spawn feed thread"));
            self.cmds = Some(tx);
        }
        self.cmds.as_ref().unwrap().send(Cmd::Start(feed)).expect("feed thread gone");
        Ok(())
    }

    /// A photo of the frames going into the encoder `dst` ([`crate::compose::Snap`]); its reply says when it is in.
    pub fn snapshot(&mut self, dst: ChnPort, s: crate::compose::Snap) -> std::result::Result<(), String> {
        self.cmds.as_ref().ok_or("no feed thread")?.send(Cmd::Snap(dst, s)).map_err(|_| "feed thread gone".to_string())
    }

    /// Whether the feed into `dst` hands DISP each canvas at its lending ([`Composer::set_early`]); nothing without that
    /// feed.
    pub fn early(&mut self, dst: ChnPort, on: bool) {
        if let Some(c) = self.cmds.as_ref() {
            let _ = c.send(Cmd::Early(dst, on));
        }
    }

    /// Stop the feed into `dst`; returns once it is gone.
    pub fn stop(&mut self, dst: ChnPort) -> std::result::Result<(), String> {
        let cmds = self.cmds.as_ref().ok_or("no feed thread")?;
        let (tx, rx) = mpsc::channel();
        cmds.send(Cmd::Stop(dst, tx)).map_err(|_| "feed thread gone")?;
        rx.recv_timeout(STOP_WAIT).map_err(|_| format!("the feed into {} did not stop in {} ms", name(dst), STOP_WAIT.as_millis()))?
    }
}

/// A feed's destination in the log.
fn name(dst: ChnPort) -> String {
    match dst.module {
        microcam_core::reconcile::Module::Disp => format!("DISP port {}", dst.chn),
        _ => format!("encoder {}/{}", dst.dev, dst.chn),
    }
}

struct Feed {
    /// where the frames go, which names the feed
    dst: ChnPort,
    port: ffi::MI_SYS_ChnPort_t,
    /// the port's poll fd; -1: no port (covered)
    fd: i32,
    way: Composer,
    /// frames taken from the port
    handed: u64,
    lost: Lost,
    on_lost: OnLost,
}

impl Feed {
    /// Every frame the port has for us, to the compositor.
    fn drain(&mut self) {
        if self.fd < 0 {
            return;
        }
        loop {
            // SAFETY: all-zero is a valid BufInfo (plain data).
            let mut info: ffi::MI_SYS_BufInfo_t = unsafe { std::mem::zeroed() };
            let mut h: ffi::MI_SYS_BUF_HANDLE = 0;
            if unsafe { ffi::MI_SYS_ChnOutputPortGetBufPa(&mut self.port, &mut info, &mut h) } != 0 {
                return;
            }
            self.way.frame(h, &info);
            self.handed += 1;
        }
    }

    /// The encoder `who` (a reader of the feed's pool) is lost: marked and told once. DISP is not: nothing builds it
    /// anew for that (the compositor logs it).
    fn lose(&self, who: ChnPort, why: &str) {
        if who.module != microcam_core::reconcile::Module::Venc {
            return;
        }
        if self.lost.mark(who.chn) {
            log::error!("feed into {}: {} {why}; the output is built anew", name(self.dst), name(who));
            (self.on_lost)(who.chn);
        }
    }

    fn stop(self) -> std::result::Result<(), String> {
        if self.fd >= 0 {
            unsafe { ffi::MI_SYS_CloseFd(self.fd) };
        }
        log::info!("feed into {} stopped: {} frames of its port", name(self.dst), self.handed);
        self.way.stop()
    }
}

fn run(rx: Receiver<Cmd>) {
    let mut feeds: Vec<Feed> = Vec::new();
    let lent = crate::canvas::Lent::open();
    if lent.is_none() {
        log::warn!("feed: the canvas module tells no lendings; DISP gets every canvas once written");
    }
    loop {
        // idle: wait for a command; feeding: take what came and go on
        let first = if feeds.is_empty() { rx.recv().ok() } else { rx.try_recv().ok() };
        let Some(first) = first else {
            if feeds.is_empty() {
                return;
            }
            step(&mut feeds, lent.as_ref());
            continue;
        };
        for cmd in std::iter::once(first).chain(rx.try_iter()) {
            match cmd {
                Cmd::Start(f) => {
                    log::info!("feed into {} started", name(f.dst));
                    feeds.push(f);
                }
                Cmd::Snap(dst, s) => match feeds.iter_mut().find(|f| f.dst == dst) {
                    Some(f) => f.way.snap(s),
                    None => {
                        let _ = s.reply.send(Err(format!("no compositor feeds {}", name(dst))));
                    }
                },
                Cmd::Early(dst, on) => {
                    if let Some(f) = feeds.iter_mut().find(|f| f.dst == dst) {
                        f.way.set_early(on && lent.is_some());
                    }
                }
                Cmd::Stop(dst, reply) => {
                    let r = match feeds.iter().position(|f| f.dst == dst) {
                        Some(i) => feeds.remove(i).stop(),
                        None => Err(format!("no feed into {}", name(dst))),
                    };
                    let _ = reply.send(r);
                }
            }
        }
        if !feeds.is_empty() {
            step(&mut feeds, lent.as_ref());
        }
    }
}

/// One round: wait for a frame on any feed's port, a lending a compositor hands DISP at, or the next covered frame due;
/// then the lendings go to their compositors and every feed drains its port.
fn step(feeds: &mut [Feed], lent: Option<&crate::canvas::Lent>) {
    let mut fds: Vec<libc::pollfd> = feeds.iter().filter(|f| f.fd >= 0).map(|f| libc::pollfd { fd: f.fd, events: libc::POLLIN, revents: 0 }).collect();
    let lent = lent.filter(|_| feeds.iter().any(|f| f.way.early()));
    if let Some(l) = lent {
        fds.push(libc::pollfd { fd: l.fd(), events: libc::POLLIN, revents: 0 });
    }
    let now = Instant::now();
    let wait = feeds
        .iter()
        .filter_map(|f| f.way.due())
        .map(|due| due.saturating_duration_since(now).as_millis() as i32)
        .fold(POLL_MS, i32::min);
    if fds.is_empty() {
        std::thread::sleep(std::time::Duration::from_millis(wait.max(0) as u64));
    } else {
        // SAFETY: pollfds we own, their count with them.
        unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, wait) };
    }
    if let Some(l) = lent.filter(|_| fds.last().is_some_and(|p| p.revents & libc::POLLIN != 0)) {
        for lending in l.read() {
            for f in feeds.iter_mut().filter(|f| f.way.early() && f.way.pool_id() == lending.pool) {
                f.way.lent(lending);
            }
        }
    }
    for f in feeds.iter_mut() {
        // a reader not known (the canvas not drawn or not named): the feed's own destination
        if let Some(who) = f.way.tick() {
            f.lose(who.unwrap_or(f.dst), "lost its canvases");
        }
        f.drain();
    }
}
