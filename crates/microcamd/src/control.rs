//! Control server: JSON lines over a Unix socket and TCP (microcam_core::protocol). One thread reads each connection and
//! answers its requests in order, a camera request as a pipeline message with a bounded wait — an action that takes time
//! (a control the pipeline builds, a recording, the player) is answered at once, its state and outcome told as events,
//! without holding the requests behind it; another writes its outgoing lines — replies
//! and, for the menu and the watchers, the events — from a bounded queue.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::rc::Rc;
use std::path::Path;
use std::sync::mpsc::{sync_channel, Sender};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use microcam_core::controls::{By, Change as Ask, Code, Refusal};
use microcam_core::formats::{FormatTable, HostStream};
use microcam_core::policy::Recording;
use microcam_core::protocol::{Envelope, Hello, Limits, ListInUse, OutputInfo, Request, ResState, Response, Snapshot, Source, PROTOCOL};
use microcam_core::wish::Wish;
use microcam_core::scene::{Kind, Node, Scene};
use microcam_core::{scene, Size};
use microcam_draw::render::Fonts;

use crate::card;
use crate::controls::{Controls, Reply};
use crate::pipeline::{ask, Msg};
use crate::res::{Change, MAX_SIDE};
use crate::followers::Client;

pub struct Control {
    pub tx: Sender<Msg>,
    pub ui: Arc<crate::ui::Ui>,
    pub formats: Arc<FormatTable>,
    pub quit: Arc<std::sync::atomic::AtomicBool>,
    pub pump: Arc<crate::uvc::pump::Shared>,
    pub player: crate::player::Player,
    pub controls: Arc<Controls>,
    pub keys: Arc<crate::keys::Keys>,
    pub running: Arc<crate::running::Running>,
    pub files: Arc<crate::files::Files>,
    /// the lists not taken from the loaded modules at start
    pub lists: Vec<ListInUse>,
}

/// A photo's size: the full picture (the first release always takes the largest).
const PHOTO: Size = Size::new(3840, 2160);

const WAIT: Duration = Duration::from_secs(15);
/// The longest request line read; a longer one closes the connection (a `ui` scene at its limits fits easily).
const MAX_LINE: u64 = 256 * 1024;
/// Lines waiting to go out on one connection; a menu that lets this many events pile up is dropped.
const OUT_QUEUE: usize = 256;
static CONNECTIONS: AtomicU64 = AtomicU64::new(0);

/// File descriptors that came with a connection's bytes (SCM_RIGHTS on the Unix socket), oldest first; each
/// `resource` with pixels takes one.
type Fds = Rc<RefCell<VecDeque<OwnedFd>>>;
/// Descriptors kept waiting for their requests; more are closed as they come.
const MAX_FDS: usize = 8;

/// A connection's stream: read by one thread, written by another, shut by the followers' list when it stops reading.
trait Stream: Read + Write + Send + Sync + Sized + 'static {
    fn dup(&self) -> std::io::Result<Self>;
    fn shut(&self);
    /// The stream to read requests from, putting descriptors that come with them into `fds`.
    fn input(self, fds: Fds) -> Box<dyn Read>;
}

impl Stream for UnixStream {
    fn dup(&self) -> std::io::Result<Self> {
        self.try_clone()
    }
    fn shut(&self) {
        let _ = self.shutdown(Shutdown::Both);
    }
    fn input(self, fds: Fds) -> Box<dyn Read> {
        Box::new(UnixIn { s: self, fds })
    }
}

impl Stream for TcpStream {
    fn dup(&self) -> std::io::Result<Self> {
        self.try_clone()
    }
    fn shut(&self) {
        let _ = self.shutdown(Shutdown::Both);
    }
    fn input(self, _: Fds) -> Box<dyn Read> {
        Box::new(self)
    }
}

/// A Unix stream read with `recvmsg`, so descriptors passed with the bytes are taken too.
struct UnixIn {
    s: UnixStream,
    fds: Fds,
}

impl Read for UnixIn {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut iov = libc::iovec { iov_base: buf.as_mut_ptr().cast(), iov_len: buf.len() };
        let mut control = [0u64; 16];
        // SAFETY: an all-zero msghdr is valid; its pointers are set to `iov` and `control`, which outlive the call.
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen = std::mem::size_of_val(&control) as _;
        // SAFETY: recvmsg writes at most the buffer's and the control area's lengths.
        let n = unsafe { libc::recvmsg(self.s.as_raw_fd(), &mut msg, libc::MSG_CMSG_CLOEXEC) };
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: the control messages are the kernel's, walked by its own macros inside `msg_controllen`; each
        // SCM_RIGHTS descriptor is new to this process and owned from here on.
        unsafe {
            let mut c = libc::CMSG_FIRSTHDR(&msg);
            while !c.is_null() {
                if (*c).cmsg_level == libc::SOL_SOCKET && (*c).cmsg_type == libc::SCM_RIGHTS {
                    let data = libc::CMSG_DATA(c) as *const i32;
                    let count = ((*c).cmsg_len as usize - libc::CMSG_LEN(0) as usize) / 4;
                    for k in 0..count {
                        let fd = OwnedFd::from_raw_fd(data.add(k).read_unaligned());
                        let mut q = self.fds.borrow_mut();
                        if q.len() < MAX_FDS {
                            q.push_back(fd);
                        }
                    }
                }
                c = libc::CMSG_NXTHDR(&msg, c);
            }
        }
        Ok(n as usize)
    }
}

/// `len` bytes at the start of the file `fd` (a memfd) handed to `f`.
fn mapped<T>(fd: &OwnedFd, len: usize, f: impl FnOnce(&[u8]) -> Result<T, String>) -> Result<T, String> {
    // SAFETY: fstat fills the struct; mmap maps the file read-only and shared, unmapped below before return.
    unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        if libc::fstat(fd.as_raw_fd(), &mut st) != 0 || (st.st_size as usize) < len {
            return Err(format!("the memfd holds {} bytes, {len} needed", st.st_size));
        }
        let p = libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_SHARED, fd.as_raw_fd(), 0);
        if p == libc::MAP_FAILED {
            return Err(format!("mmap: {}", std::io::Error::last_os_error()));
        }
        let r = f(std::slice::from_raw_parts(p as *const u8, len));
        libc::munmap(p, len);
        r
    }
}

impl Control {
    /// One request from `client`; None when the reply went out already.
    fn handle(&self, client: &Client, fds: &Fds, seq: Option<u64>, req: Request) -> Option<Response> {
        Some(match req {
            Request::Resource { .. } | Request::Release { .. } if !self.ui.followers.is_menu(client.conn) => Response::err("resources are the menu's: hello first"),
            Request::Resource { id, source: Source::Pixels { w, h } } => {
                let Some(fd) = fds.borrow_mut().pop_front() else {
                    return Some(Response::err("resource: pixels come in a memfd passed with the request on the Unix socket"));
                };
                let len = w as usize * h as usize * 4;
                match mapped(&fd, len, |px| self.ui.res.pixels(&id, w, h, px)) {
                    Ok(()) => {
                        send(client, Response { seq, ..Response::ok() });
                        self.ui.res_changed(Change::State(id, ResState::Ready { w, h }));
                        return None;
                    }
                    Err(e) => Response::err(format!("resource: {e}")),
                }
            }
            Request::Resource { id, source: Source::File(path) } => match self.ui.res.file(&id, &path) {
                Ok(()) => {
                    send(client, Response { seq, ..Response::ok() });
                    self.ui.res_changed(Change::State(id.clone(), ResState::Loading));
                    self.ui.res.load(&id);
                    return None;
                }
                Err(e) => Response::err(format!("resource: {e}")),
            },
            Request::Resource { id, source: Source::Camera { w, h } } => {
                if w < 16 || h < 2 || w > MAX_SIDE || h > MAX_SIDE {
                    return Some(Response::err(format!("resource: camera {w}x{h}: each side 16..{MAX_SIDE}")));
                }
                match self.ui.res.camera(&id) {
                    Ok(picture) => {
                        send(client, Response { seq, ..Response::ok() });
                        self.ui.res_changed(Change::State(id, ResState::Loading));
                        let _ = self.tx.send(Msg::Grab { picture, size: Size::new(w, h) });
                        return None;
                    }
                    Err(e) => Response::err(format!("resource: {e}")),
                }
            }
            Request::Release { id } => match self.ui.res.release(&id) {
                Ok(()) => Response::ok(),
                Err(e) => Response::err(format!("release: {e}")),
            },
            Request::Hello { take } => {
                let reply = |outputs| line(Response { seq, hello: Some(Hello { protocol: PROTOCOL, scene: scene::VERSION, limits: limits() }), snapshot: Some(self.snapshot(outputs)), ..Response::ok() });
                match self.ui.subscribe(client.clone(), Some(take), reply) {
                    Ok(()) => return None,
                    Err(e) => Response::err(e),
                }
            }
            Request::Watch => {
                let reply = |outputs| line(Response { seq, snapshot: Some(self.snapshot(outputs)), ..Response::ok() });
                let _ = self.ui.subscribe(client.clone(), None, reply);
                return None;
            }
            Request::Set { name, value } => {
                self.controls.change(&name, Ask::Set(value), By::User, answer(client, seq));
                return None;
            }
            Request::Adjust { name, steps } => {
                self.controls.change(&name, Ask::Adjust(steps), By::User, answer(client, seq));
                return None;
            }
            // the owner checked and the record stored as one, so a `reset` taking the scene comes wholly before or after
            Request::Keep { record } => match self.ui.followers.as_menu(client.conn, || self.controls.keep(record)) {
                None => Response::err("keep: the scene's owner's only (hello first)"),
                Some(Ok(())) => Response::ok(),
                Some(Err(e)) => Response::refused(Refusal::new(Code::Invalid, e)),
            },
            Request::Key { key, phase } => match self.keys.key(client.conn, key, phase) {
                Ok(()) => Response::ok(),
                Err(r) => Response::refused(r),
            },
            Request::Photo => {
                let path = card::photo(self.files.next());
                match self.running.photo(&path.to_string_lossy(), self.player.info().is_some()) {
                    Ok(()) => {
                        if self.tx.send(Msg::Photo { size: PHOTO, path }).is_err() {
                            self.running.photo_done(Err(Refusal::new(Code::Refused, "the pipeline thread is gone")));
                        }
                        Response::ok()
                    }
                    Err(r) => Response::refused(r),
                }
            }
            Request::Record => {
                let path = card::video(self.files.next()).to_string_lossy().into_owned();
                match self.running.record(&path, self.player.info().is_some()) {
                    Ok(()) => {
                        let w = Wish::Record(Recording { size: self.controls.record_size(), path });
                        if self.tx.send(Msg::Wish(w, None)).is_err() {
                            self.running.record_built(Err(Refusal::new(Code::Refused, "the pipeline thread is gone")));
                        }
                        Response::ok()
                    }
                    Err(r) => Response::refused(r),
                }
            }
            Request::RecordStop => match self.running.record_stop() {
                Ok(()) => {
                    if self.tx.send(Msg::Wish(Wish::RecordStop, None)).is_err() {
                        self.running.record_stop_failed("the pipeline thread is gone".into());
                    }
                    Response::ok()
                }
                Err(r) => Response::refused(r),
            },
            Request::Format => match self.running.format(self.player.info().is_some()) {
                Ok(()) => {
                    let running = self.running.clone();
                    if let Err(e) = thread::Builder::new().name("format".into()).spawn(move || running.format_done(card::format())) {
                        self.running.format_done(Err(format!("no thread to format: {e}")));
                    }
                    Response::ok()
                }
                Err(r) => Response::refused(r),
            },
            Request::Files { from, count } => Response { files: Some(self.files.window(from, count)), ..Response::ok() },
            Request::Reset => match self.controls.reset(|| self.ui.take_scene()) {
                Ok(()) => Response::ok(),
                Err(r) => Response::refused(r),
            },
            Request::Measure { text } => {
                static FONTS: std::sync::OnceLock<Fonts> = std::sync::OnceLock::new();
                // checked as a scene's text node would be
                let node = Node { id: None, level: None, visible: true, paused: false, kind: Kind::Text(text) };
                let s = Scene { nodes: vec![node], ..Scene::empty() };
                if let Err(e) = s.validate() {
                    return Some(Response::err(format!("measure: {e}")));
                }
                let Kind::Text(text) = &s.nodes[0].kind else { unreachable!("built as text") };
                Response { measure: Some(microcam_draw::render::measure(text, FONTS.get_or_init(Fonts::new))), ..Response::ok() }
            }
            Request::Commit { base, ops } => {
                // queued without waiting: it is given under the `ui`'s lock
                let reply = |rev| client.queue(line(Response { seq, revision: Some(rev), ..Response::ok() }));
                match self.ui.commit(client.conn, base, &ops, reply) {
                    Ok(_) => return None,
                    Err(e) => Response::err(format!("commit: {e}")),
                }
            }
            // answered at once, the player starting or stopping meanwhile (`player`), the outcome told
            Request::PlayStop => match self.player.stop() {
                Ok(()) => {
                    if self.tx.send(Msg::Wish(Wish::PlayStop, None)).is_err() {
                        self.player.stop_done(Err(Refusal::new(Code::Refused, "the pipeline thread is gone")));
                    }
                    Response::ok()
                }
                Err(r) => Response::refused(r),
            },
            Request::Play { file } => match self.player.play(&file, || self.running.play_free()) {
                Ok(()) => {
                    if self.tx.send(Msg::Play { file }).is_err() {
                        self.player.play_done(Err(Refusal::new(Code::Refused, "the pipeline thread is gone")));
                    }
                    Response::ok()
                }
                Err(r) => Response::refused(r),
            },
            Request::PlayPause => flat(Ok(self.player.pause())),
            Request::PlayResume => flat(Ok(self.player.resume())),
            Request::PlaySeek { ms } => flat(Ok(self.player.seek(ms))),
            Request::Account => match ask(&self.tx, Msg::Account, WAIT) {
                Ok(priced) => Response { account: Some(crate::mem::account(priced)), ..Response::ok() },
                Err(e) => Response::err(e),
            },
            Request::Price { with } => match self.price_of(with) {
                Ok((wishes, host)) => match ask(&self.tx, |reply| Msg::Price { wishes, host, reply }, WAIT) {
                    Ok(Ok(price)) => Response { price: Some(price), ..Response::ok() },
                    Ok(Err(e)) | Err(e) => Response::err(format!("price: {e}")),
                },
                Err(e) => Response::err(format!("price: {e}")),
            },
            Request::UiWindows => Response { lines: Some(self.ui.windows()), ..Response::ok() },
            Request::Mem { reset, brief } => {
                let priced = if brief { None } else { ask(&self.tx, Msg::Account, WAIT).ok() };
                Response { lines: Some(crate::mem::report(reset, brief, priced)), ..Response::ok() }
            }
            Request::Status => match ask(&self.tx, Msg::Status, WAIT) {
                Ok(mut s) => {
                    let p = self.pump.st.lock().unwrap();
                    s.pump = Some(p.since_start());
                    s.usb_delay = p.delay.last;
                    s.build = crate::BUILD["microcam-build ".len()..].to_string();
                    s.scene = self.ui.has_scene();
                    s.player = self.player.info();
                    s.card = card::info();
                    s.lists = self.lists.clone();
                    Response { status: Some(s), ..Response::ok() }
                }
                Err(e) => Response::err(e),
            },
            Request::SimStream { off: true, .. } => match ask(&self.tx, Msg::StreamOff, WAIT) {
                Ok(()) => Response::ok(),
                Err(e) => Response::err(e),
            },
            Request::SimStream { format, frame, interval, .. } => match self.formats.resolve(format, frame, interval) {
                Ok(h) => match ask(&self.tx, |r| Msg::StreamOn(h, r), WAIT) {
                    Ok(Ok(epoch)) => Response::msg(format!("epoch {}", epoch.0)),
                    Ok(Err(e)) | Err(e) => Response::err(e),
                },
                Err(e) => Response::err(e),
            },
            Request::Rebuild => flat(ask(&self.tx, Msg::Rebuild, WAIT)),
            Request::Log { lines } => Response { lines: Some(crate::logger::tail(lines)), ..Response::ok() },
            Request::LogSince { from } => {
                let (lines, next) = crate::logger::since(from);
                Response { lines: Some(lines), message: Some(next.to_string()), ..Response::ok() }
            }
            Request::LogLevel { spec } => match spec.map_or(Ok(()), |s| crate::logger::set_level(&s)) {
                Ok(()) => Response::msg(crate::logger::current()),
                Err(e) => Response::err(e),
            },
            Request::Quit => {
                self.quit.store(true, Ordering::SeqCst);
                Response::msg("stopping")
            }
        })
    }

    /// What a client needs to draw the camera's state, beside the outputs the `ui` reads under its lock.
    fn snapshot(&self, outputs: Vec<OutputInfo>) -> Snapshot {
        Snapshot { controls: self.controls.all(), running: self.running.info(), files: self.files.info(), outputs, player: self.player.info(), card: card::info(), record: self.controls.record() }
    }

    /// The wish a request changes: a recording at `size` (its file set by whoever starts it), its stop, the player's
    /// stop, a built control's `set`.
    fn wish_of(&self, req: &Request, size: Size) -> Result<Wish, String> {
        Ok(match req {
            Request::Record => Wish::Record(Recording { size, path: String::new() }),
            Request::RecordStop => Wish::RecordStop,
            Request::PlayStop => Wish::PlayStop,
            Request::Set { name, value } => self.controls.price_wish(name, value.clone()).map_err(|r| r.error)?,
            r => return Err(format!("{} changes no wish", serde_json::to_string(r).unwrap_or_default())),
        })
    }

    /// `price`'s requests: the wishes in order, and the host stream when a `sim-stream` replaces it. A `set` of
    /// `record-size` changes the size of the `record`s after it.
    fn price_of(&self, with: Vec<Request>) -> Result<(Vec<Wish>, Option<Option<HostStream>>), String> {
        let mut wishes = Vec::new();
        let mut host = None;
        let mut size = self.controls.record_size();
        for req in with {
            match req {
                Request::Set { name, value } if name == "record-size" => {
                    let v = self.controls.target(&name, Ask::Set(value)).map_err(|r| r.error)?;
                    size = self.controls.record_size_of(&v).ok_or("record-size: no size")?;
                }
                Request::SimStream { off: true, .. } => host = Some(None),
                Request::SimStream { format, frame, interval, .. } => host = Some(Some(self.formats.resolve(format, frame, interval).map_err(|e| e.to_string())?)),
                r => wishes.push(self.wish_of(&r, size)?),
            }
        }
        Ok((wishes, host))
    }

    fn serve<S: Stream>(self: &Arc<Self>, stream: S) {
        let conn = CONNECTIONS.fetch_add(1, Ordering::SeqCst) + 1;
        let (Ok(mut out), Ok(shut)) = (stream.dup(), stream.dup()) else {
            return;
        };
        let (tx, rx) = sync_channel::<String>(OUT_QUEUE);
        let writer = thread::Builder::new().name("ctl-out".into()).spawn(move || {
            for mut l in rx {
                l.push('\n');
                if out.write_all(l.as_bytes()).is_err() {
                    break;
                }
            }
        });
        if writer.is_err() {
            return;
        }
        let client = Client { conn, tx, close: Arc::new(move || shut.shut()) };
        self.serve_conn(&client, stream);
        self.keys.gone(conn);
        self.ui.gone(conn);
        (client.close)();
    }

    fn serve_conn<S: Stream>(self: &Arc<Self>, client: &Client, stream: S) {
        let fds = Fds::default();
        let mut reader = BufReader::new(stream.input(fds.clone()));
        let mut line = String::new();
        loop {
            line.clear();
            match (&mut reader).take(MAX_LINE).read_line(&mut line) {
                Ok(0) | Err(_) => return,
                Ok(n) if n as u64 == MAX_LINE && !line.ends_with('\n') => {
                    log::warn!("control: a request longer than {MAX_LINE} B, connection closed");
                    return;
                }
                Ok(_) => {}
            }
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            let resp = match serde_json::from_str::<Envelope>(t) {
                Ok(Envelope { seq, req }) => self.handle(client, &fds, seq, req).map(|r| Response { seq, ..r }),
                Err(e) => Some(Response::err(format!("bad request: {e}"))),
            };
            if let Some(r) = resp {
                if !send(client, r) {
                    return;
                }
            }
            if self.quit.load(Ordering::SeqCst) {
                return;
            }
        }
    }

    pub fn listen(self: Arc<Self>, unix: &Path, tcp: &str) -> std::io::Result<()> {
        let _ = std::fs::remove_file(unix);
        let ul = UnixListener::bind(unix)?;
        let me = self.clone();
        thread::Builder::new()
            .name("ctl-unix".into())
            .spawn(move || {
                for s in ul.incoming().flatten() {
                    let me = me.clone();
                    thread::spawn(move || me.serve(s));
                }
            })?;
        if !tcp.is_empty() {
            let tl = TcpListener::bind(tcp)?;
            let me = self.clone();
            thread::Builder::new()
                .name("ctl-tcp".into())
                .spawn(move || {
                    for s in tl.incoming().flatten() {
                        // replies and events are small lines that follow each other: never held back for an ACK
                        let _ = s.set_nodelay(true);
                        let me = me.clone();
                        thread::spawn(move || me.serve(s));
                    }
                })?;
        }
        Ok(())
    }
}

/// A reply onto the connection's queue (waiting while it is full: it is the client's own request); false when the
/// connection is gone.
fn send(client: &Client, r: Response) -> bool {
    client.tx.send(line(r)).is_ok()
}

fn line(r: Response) -> String {
    serde_json::to_string(&r).unwrap_or_else(|_| "{\"ok\":false}".into())
}

/// A `set` or `adjust` answered onto the connection's queue, from whichever thread has the answer (without waiting:
/// the pipeline's thread may give it).
fn answer(client: &Client, seq: Option<u64>) -> Reply {
    let client = client.clone();
    Box::new(move |r| client.queue(line(Response { seq, ..Response::of(r) })))
}

/// What the menu is told it may ask.
fn limits() -> Limits {
    Limits {
        nodes: scene::MAX_NODES,
        depth: scene::MAX_DEPTH,
        segments: scene::MAX_SEGMENTS,
        text: scene::MAX_TEXT,
        text_total: scene::MAX_TEXT_TOTAL,
        levels: scene::MAX_LEVEL,
        menu_bytes: mi::layer::share(),
        picture_side: MAX_SIDE,
    }
}

fn flat(r: Result<Result<(), String>, String>) -> Response {
    match r {
        Ok(Ok(())) => Response::ok(),
        Ok(Err(e)) | Err(e) => Response::err(e),
    }
}
