//! usage: microcam-menu [--take] [-v] [ADDR]
//!   ADDR: host:port for TCP (from a laptop: 192.168.1.100:7700) or the control socket's path (default /tmp/microcam.sock)
//!   --take: the scene taken from another menu (a program started by hand); without it the menu waits for the scene
//!   -v: a line for each step shown, with its times
//!   exits 1 when microcamd speaks another protocol or its snapshot cannot be read
//!
//! The shell: the socket, the clock, one loop. A loop turn that hangs ends the process (`alarm`), a broken socket is
//! opened again; the core and the presenter do the rest.

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use microcam_core::keys::Phase;
use microcam_core::protocol::{Envelope, Event, ResState, Response, PROTOCOL};
use microcam_menu::core::{Core, Effect, Input};
use microcam_menu::presenter::{Presenter, SEQ_BASE};

/// A turn's longest wait for the socket.
const TURN_MS: u64 = 1000;
/// A turn that takes this long has hung: the process ends and the boot script's loop starts it again.
const ALARM_S: u32 = 10;
/// The pause before the socket is tried again.
const AGAIN: Duration = Duration::from_secs(1);
const CONNECT: Duration = Duration::from_secs(2);

enum Conn {
    Tcp(TcpStream),
    Unix(UnixStream),
}

impl Conn {
    fn open(addr: &str) -> std::io::Result<Conn> {
        if addr.contains(':') && !addr.starts_with('/') {
            let a = addr.to_socket_addrs()?.next().ok_or_else(|| std::io::Error::new(ErrorKind::NotFound, "no address"))?;
            let s = TcpStream::connect_timeout(&a, CONNECT)?;
            s.set_nodelay(true)?;
            Ok(Conn::Tcp(s))
        } else {
            Ok(Conn::Unix(UnixStream::connect(addr)?))
        }
    }

    fn wait(&self, ms: u64) -> std::io::Result<()> {
        let t = Some(Duration::from_millis(ms.max(1)));
        match self {
            Conn::Tcp(s) => s.set_read_timeout(t),
            Conn::Unix(s) => s.set_read_timeout(t),
        }
    }

    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Conn::Tcp(s) => s.read(b),
            Conn::Unix(s) => s.read(b),
        }
    }

    fn write(&mut self, b: &[u8]) -> std::io::Result<()> {
        match self {
            Conn::Tcp(s) => s.write_all(b),
            Conn::Unix(s) => s.write_all(b),
        }
    }
}

struct Shell {
    core: Core,
    pres: Presenter,
    start: Instant,
    /// when the core and the presenter want to be asked again
    core_wake: Option<u64>,
    pres_wake: Option<u64>,
    /// the last refusal said
    refused: Option<String>,
}

impl Shell {
    fn now(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    fn say(&self, line: &str) {
        eprintln!("{:>9.3} {line}", self.now() as f64 / 1000.0);
    }

    /// An input to the core, then the presenter brought up to the state; the requests written, whether there were any.
    fn feed(&mut self, conn: &mut Conn, input: Input) -> std::io::Result<bool> {
        let now = self.now();
        let mut out: Vec<Envelope> = Vec::new();
        self.core_wake = None;
        for fx in self.core.step(now, input) {
            match fx {
                Effect::Send(e) => out.push(e),
                Effect::SendClear(e) => self.pres.later(now, e),
                Effect::Wake(at) => self.core_wake = Some(at),
            }
        }
        self.present(conn, out)
    }

    fn present(&mut self, conn: &mut Conn, mut out: Vec<Envelope>) -> std::io::Result<bool> {
        let (reqs, wake) = self.pres.update(self.now(), &self.core);
        out.extend(reqs);
        self.pres_wake = wake;
        for n in std::mem::take(&mut self.pres.notes) {
            self.say(&n);
        }
        let any = !out.is_empty();
        for e in out {
            let mut l = serde_json::to_string(&e).expect("a request is JSON");
            l.push('\n');
            conn.write(l.as_bytes())?;
        }
        Ok(any)
    }

    fn line(&mut self, conn: &mut Conn, line: &str) -> std::io::Result<()> {
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                self.say(&format!("not JSON from microcamd: {e}"));
                return Ok(());
            }
        };
        if v.get("event").is_some() {
            let e: Event = match serde_json::from_value(v) {
                Ok(e) => e,
                Err(e) => {
                    self.say(&format!("an event this menu does not know: {e}"));
                    return Ok(());
                }
            };
            let now = self.now();
            let key = matches!(&e, Event::Key { phase, .. } if *phase != Phase::Release);
            if let Event::Key { key, phase } = &e {
                if *phase != Phase::Release {
                    self.pres.key(now, format!("{key:?} {phase:?}").to_lowercase());
                }
            }
            if let Event::Lost = &e {
                self.say("the scene went to another menu");
            }
            if self.pres.event(now, &e) {
                return self.present(conn, Vec::new()).map(|_| ());
            }
            let mut e = match e {
                Event::Resource { id, state } => match self.pres.core_id(&id) {
                    Some(core_id) => Event::Resource { id: core_id.to_string(), state },
                    None => return Ok(()),
                },
                e => e,
            };
            match &mut e {
                Event::Failed { holders, .. } | Event::NotShown { holders, .. } | Event::Resource { state: ResState::Failed { holders, .. }, .. } => self.pres.core_holders(holders),
                _ => {}
            }
            let asked = self.feed(conn, Input::Event(e))?;
            if key && !asked {
                self.pres.unkey();
            }
            return Ok(());
        }
        let whole = v.get("hello").is_some() || v.get("snapshot").is_some();
        let r: Response = match serde_json::from_value(v) {
            Ok(r) => r,
            // the camera's state unread is no menu: out, for the start's loop to try again
            Err(e) if whole => self.quit(&format!("microcamd's snapshot is not one this menu reads (protocol {PROTOCOL}): {e}")),
            Err(e) => {
                self.say(&format!("a reply this menu does not read: {e}"));
                return Ok(());
            }
        };
        if let Some(h) = r.hello.as_ref().filter(|h| h.protocol != PROTOCOL) {
            self.quit(&format!("microcamd speaks protocol {}, this menu {PROTOCOL}", h.protocol));
        }
        if r.seq.is_some_and(|s| s > SEQ_BASE) {
            self.pres.reply(&r);
            return self.present(conn, Vec::new()).map(|_| ());
        }
        if let Some(h) = &r.hello {
            self.say(&format!("the scene is ours: protocol {}, scene format {}", h.protocol, h.scene));
        } else if !r.ok {
            // the same refusal again (the scene asked for each second) is not said again
            let why = format!("refused: {}", r.error.as_deref().unwrap_or(""));
            if self.refused.as_deref() != Some(why.as_str()) {
                self.say(&why);
                self.refused = Some(why);
            }
        }
        if r.ok {
            self.refused = None;
        }
        let mut r = r;
        self.pres.core_holders(&mut r.holders);
        self.feed(conn, Input::Reply(r)).map(|_| ())
    }

    /// Out with an error, the reason in the log.
    fn quit(&self, why: &str) -> ! {
        self.say(why);
        std::process::exit(1)
    }

    /// One connection, until it breaks.
    fn run(&mut self, conn: &mut Conn) -> std::io::Result<()> {
        self.feed(conn, Input::Connected)?;
        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            // SAFETY: alarm only sets the process's timer.
            unsafe { libc::alarm(ALARM_S) };
            let now = self.now();
            let next = [self.core_wake, self.pres_wake].into_iter().flatten().min();
            conn.wait(next.map_or(TURN_MS, |t| t.saturating_sub(now)).min(TURN_MS))?;
            match conn.read(&mut chunk) {
                Ok(0) => return Err(std::io::Error::new(ErrorKind::UnexpectedEof, "microcamd closed the socket")),
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted) => {}
                Err(e) => return Err(e),
            }
            while let Some(i) = buf.iter().position(|b| *b == b'\n') {
                let l: Vec<u8> = buf.drain(..=i).collect();
                let l = String::from_utf8_lossy(&l);
                if !l.trim().is_empty() {
                    self.line(conn, l.trim())?;
                }
            }
            let now = self.now();
            if self.core_wake.is_some_and(|t| t <= now) {
                self.feed(conn, Input::Clock)?;
            } else if self.pres_wake.is_some_and(|t| t <= now) {
                self.present(conn, Vec::new())?;
            }
        }
    }
}

fn main() {
    let mut take = false;
    let mut steps = false;
    let mut addr = "/tmp/microcam.sock".to_string();
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--take" => take = true,
            "-v" => steps = true,
            "-h" | "--help" => {
                eprintln!("usage: microcam-menu [--take] [-v] [ADDR]   (ADDR: host:port, or the socket's path; /tmp/microcam.sock)");
                return;
            }
            _ => addr = a,
        }
    }
    let mut pres = Presenter::new();
    pres.steps = steps;
    let mut sh = Shell { core: Core::new(take), pres, start: Instant::now(), core_wake: None, pres_wake: None, refused: None };
    loop {
        // SAFETY: alarm only sets the process's timer.
        unsafe { libc::alarm(ALARM_S) };
        match Conn::open(&addr) {
            Ok(mut conn) => {
                sh.say(&format!("connected to {addr}"));
                if let Err(e) = sh.run(&mut conn) {
                    sh.say(&format!("the link broke: {e}"));
                }
                // nothing of the session is sent again; the presenter starts a new one at the next hello
                let now = sh.now();
                sh.core.step(now, Input::Disconnected);
                sh.pres.update(now, &sh.core);
            }
            Err(e) => sh.say(&format!("{addr}: {e}")),
        }
        std::thread::sleep(AGAIN);
    }
}
