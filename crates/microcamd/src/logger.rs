//! Logging for development: every line into a ring in RAM (1 MiB, nothing on flash), the kernel's messages beside
//! them, the level changed live (`microcam log-level`), new lines followed from the laptop (`microcam log follow`), the ring's
//! tail written to flash once when microcamd panics or its watchdog stops feeding. Meant to go whole once the active phase
//! is over: every line is a plain `log` macro (a build with the `log` crate's `max_level_off` feature drops them all)
//! and the rest lives in this file.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::{Mutex, OnceLock, RwLock};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use log::LevelFilter;

/// The ring's size: the longest look back.
const RING_BYTES: usize = 1 << 20;
/// How much of the ring's tail a crash writes to flash.
const DUMP_BYTES: usize = 64 << 10;
const DUMP_PATH: &str = "/customer/microcam-crash.log";

pub struct Logger {
    ring: Mutex<Ring>,
    /// the level for everything, then per part of microcamd (the last segment of the target: `ui`, `pipeline`, `kernel` …)
    spec: RwLock<(LevelFilter, Vec<(String, LevelFilter)>)>,
}

#[derive(Default)]
struct Ring {
    lines: VecDeque<(u64, String)>,
    bytes: usize,
    /// the number of the next line
    next: u64,
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

/// Start logging at `spec` (as `microcam log-level` takes it) and read the kernel's messages into the same ring.
pub fn init(spec: &str) {
    let logger = Logger { ring: Mutex::default(), spec: RwLock::new((LevelFilter::Info, Vec::new())) };
    if LOGGER.set(logger).is_ok() {
        log::set_logger(LOGGER.get().unwrap()).ok();
        if let Err(e) = set_level(spec) {
            eprintln!("microcamd: log level {spec:?}: {e}; info");
            let _ = set_level("info");
        }
        let _ = thread::Builder::new().name("kmsg".into()).spawn(kmsg);
        std::panic::set_hook(Box::new(|p| {
            log::error!("panic: {p}");
            dump("panic");
        }));
    }
}

fn level(s: &str) -> Result<LevelFilter, String> {
    s.parse().map_err(|_| format!("{s:?} is not off, error, warn, info, debug or trace"))
}

/// `debug`, `off`, or a level then parts: `info,ui=debug,kernel=off`.
pub fn set_level(spec: &str) -> Result<(), String> {
    let mut all = LevelFilter::Info;
    let mut parts = Vec::new();
    for p in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        match p.split_once('=') {
            Some((name, l)) => parts.push((name.to_string(), level(l)?)),
            None => all = level(p)?,
        }
    }
    let max = parts.iter().map(|p| p.1).fold(all, std::cmp::max);
    let l = LOGGER.get().ok_or("no logger")?;
    *l.spec.write().unwrap() = (all, parts);
    log::set_max_level(max);
    Ok(())
}

/// The level as `set_level` takes it.
pub fn current() -> String {
    LOGGER
        .get()
        .map(|l| {
            let s = l.spec.read().unwrap();
            std::iter::once(s.0.to_string().to_lowercase()).chain(s.1.iter().map(|(n, v)| format!("{n}={}", v.to_string().to_lowercase()))).collect::<Vec<_>>().join(",")
        })
        .unwrap_or_default()
}

impl Logger {
    fn allows(&self, part: &str, l: log::Level) -> bool {
        let s = self.spec.read().unwrap();
        l <= s.1.iter().find(|(n, _)| n == part).map_or(s.0, |p| p.1)
    }

    fn push(&self, line: String) {
        let mut r = self.ring.lock().unwrap();
        r.bytes += line.len();
        let n = r.next;
        r.next += 1;
        r.lines.push_back((n, line));
        while r.bytes > RING_BYTES {
            match r.lines.pop_front() {
                Some((_, l)) => r.bytes -= l.len(),
                None => break,
            }
        }
    }
}

fn part(target: &str) -> &str {
    target.rsplit("::").next().unwrap_or("")
}

impl log::Log for Logger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        self.allows(part(m.target()), m.level())
    }

    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let line = format!("{} {:5} {} {}", stamp(), r.level(), part(r.target()), r.args());
        // what goes wrong also reaches stderr (microcamd's boot log in RAM), for a daemon that dies before anyone reads the ring
        if r.level() <= log::Level::Warn {
            eprintln!("{line}");
        }
        self.push(line);
    }

    fn flush(&self) {}
}

fn stamp() -> String {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let s = d.as_secs();
    format!("{:02}:{:02}:{:02}.{:03}", (s / 3600) % 24, (s / 60) % 60, s % 60, d.subsec_millis())
}

/// The last `n` lines.
pub fn tail(n: usize) -> Vec<String> {
    LOGGER.get().map(|l| l.ring.lock().unwrap().lines.iter().rev().take(n).rev().map(|(_, s)| s.clone()).collect()).unwrap_or_default()
}

/// The lines from number `from` on (all the ring holds when it has moved past it) and the number of the next one.
pub fn since(from: u64) -> (Vec<String>, u64) {
    let Some(l) = LOGGER.get() else {
        return (Vec::new(), 0);
    };
    let r = l.ring.lock().unwrap();
    (r.lines.iter().filter(|(n, _)| *n >= from).map(|(_, s)| s.clone()).collect(), r.next)
}

/// The ring's tail to flash, once per call: a crash's last words (`reason` heads the file). One NOR write.
pub fn dump(reason: &str) {
    let Some(l) = LOGGER.get() else {
        return;
    };
    // a panic while the ring is held must not hang the hook
    let Ok(r) = l.ring.try_lock() else {
        return;
    };
    let mut tail: Vec<&str> = Vec::new();
    let mut size = 0;
    for (_, s) in r.lines.iter().rev() {
        size += s.len() + 1;
        if size > DUMP_BYTES {
            break;
        }
        tail.push(s);
    }
    let Ok(mut f) = std::fs::File::create(DUMP_PATH) else {
        return;
    };
    let _ = writeln!(f, "{} microcamd: {reason}", stamp());
    for s in tail.iter().rev() {
        let _ = writeln!(f, "{s}");
    }
    write_threads(&mut f);
    let _ = f.sync_all();
}

/// Where each of microcamd's threads is in the kernel: name, state, wait channel, the syscall line (number and
/// arguments; for an ioctl, 54 on ARM, the second is the command) and, for an ioctl, the device its fd is.
fn write_threads(f: &mut std::fs::File) {
    let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
        return;
    };
    let _ = writeln!(f, "threads at the dump (name state wchan | syscall | fd):");
    for t in tasks.flatten() {
        let p = t.path();
        let read = |n: &str| std::fs::read_to_string(p.join(n)).unwrap_or_default().trim().to_string();
        let stat = read("stat");
        let state = stat.rsplit(") ").next().and_then(|s| s.split(' ').next()).unwrap_or("?").to_string();
        let sys = read("syscall");
        let mut it = sys.split(' ');
        let fd = match (it.next(), it.next()) {
            (Some("54"), Some(fd)) => i64::from_str_radix(fd.trim_start_matches("0x"), 16)
                .ok()
                .and_then(|n| std::fs::read_link(format!("/proc/self/fd/{n}")).ok())
                .map(|l| l.display().to_string())
                .unwrap_or_default(),
            _ => String::new(),
        };
        let _ = writeln!(f, "  {} {state} {} | {sys} | {fd}", read("comm"), read("wchan"));
    }
}

/// Every 10 s at debug, part `health`: the USB frames a second and those dropped (`usb()` counts both), memory, load,
/// and the memory-to-memory scaler's (SCL dev 3) busy time per user in ms a second.
pub fn health(usb: impl Fn() -> (u64, u64) + Send + 'static) {
    let _ = thread::Builder::new().name("health".into()).spawn(move || {
        let mut last = usb();
        let mut scl = mi::scl::busy_us();
        loop {
            thread::sleep(std::time::Duration::from_secs(10));
            let now = usb();
            let scl_now = mi::scl::busy_us();
            if !log::log_enabled!(target: "health", log::Level::Debug) {
                last = now;
                scl = scl_now;
                continue;
            }
            let busy: Vec<String> = mi::scl::User::ALL.iter().zip(scl_now.iter().zip(scl)).map(|(u, (n, l))| format!("{} {}", u.name(), n.saturating_sub(l) / 10_000)).collect();
            scl = scl_now;
            let mem = std::fs::read_to_string("/proc/meminfo").ok().and_then(|m| m.lines().find(|l| l.starts_with("MemAvailable")).map(|l| l.split_whitespace().nth(1).unwrap_or("?").to_string()));
            let load = std::fs::read_to_string("/proc/loadavg").ok().and_then(|l| l.split_whitespace().next().map(str::to_string));
            log::debug!(target: "health", "usb {:.1} fps, {} dropped; MemAvailable {} kB; load {}; SCL dev 3 ms/s: {}", now.0.saturating_sub(last.0) as f64 / 10.0, now.1.saturating_sub(last.1), mem.unwrap_or_default(), load.unwrap_or_default(), busy.join(", "));
            last = now;
        }
    });
}

/// The kernel's messages, from boot on, into the ring as part `kernel` (its own clock in brackets).
fn kmsg() {
    let Ok(mut f) = std::fs::File::open("/dev/kmsg") else {
        log::warn!("kmsg: /dev/kmsg not readable; no kernel lines");
        return;
    };
    let mut buf = vec![0u8; 8192];
    loop {
        // one record per read: "prio,seq,usecs,flags;text\n" (continuation lines start with a space)
        let n = match f.read(&mut buf) {
            Ok(n) => n,
            // records overwritten before we read them: carry on from the oldest left
            Err(e) if e.raw_os_error() == Some(libc::EPIPE) => continue,
            Err(_) => return,
        };
        let rec = String::from_utf8_lossy(&buf[..n]);
        let Some((head, text)) = rec.split_once(';') else {
            continue;
        };
        let mut h = head.split(',');
        let prio: u32 = h.next().and_then(|v| v.parse().ok()).unwrap_or(6);
        let usecs: u64 = h.nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
        let lvl = match prio & 7 {
            0..=3 => log::Level::Error,
            4 => log::Level::Warn,
            5 | 6 => log::Level::Info,
            _ => log::Level::Debug,
        };
        let Some(l) = LOGGER.get() else {
            return;
        };
        if !l.allows("kernel", lvl) {
            continue;
        }
        let text = text.lines().next().unwrap_or("").trim_end();
        l.push(format!("{} {:5} kernel [{:5}.{:03}] {text}", stamp(), lvl, usecs / 1_000_000, usecs / 1000 % 1000));
    }
}
