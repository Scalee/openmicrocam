//! The outputs' canvases, owned by our kernel module `microcam_canvas` (`kmods/microcam-canvas`): a pool of canvas-sized NV12 frames per output. The module lends the feed's SCL port views of free canvases
//! (the live window's rectangle at the canvas's stride), so the port writes the live picture straight into them, and
//! gives each of the pool's readers — an encoder's input port, DISP's input port 0 — the canvas microcamd names ([`hand`])
//! through that input port's allocator; it sees each reader let the canvas go itself. Drawing into a canvas (pictures,
//! black, the `ui`'s patches) goes through [`take`] and [`give`].
//!
//! One command file for the process, kept open for its life: if microcamd dies the module leaves the pools to mi_sys's own
//! teardown. A pool is made after its readers and before its port's first frame, and closed (or a reader taken off)
//! while the port's channel and the readers still exist (mi_sys's setter BUGs on a destroyed channel).

use std::fs::File;
use std::io::{Read, Write};
use std::sync::Mutex;

use microcam_core::reconcile::{ChnPort, Module};
use microcam_core::Size;
use mi_sys as ffi;

use crate::gfx::Nv12;
use crate::sys::{chn_port, module_id};
use crate::{check, Result, PIXEL_NV12};

const PATH: &str = "/proc/microcam_canvas";
/// The port writes a window only on this grid of columns.
pub const GRID: u32 = 16;

/// A pool as the module made it: the canvases' physical addresses and their layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pool {
    pub id: u32,
    pub size: Size,
    /// rows allocated per plane (≥ `size.h`; H.264 reads whole blocks of 32)
    pub rows: u32,
    pub canvases: Vec<u64>,
    /// the input ports canvases are handed to, each at its own rate
    pub readers: Vec<Reader>,
    /// readers handed only the canvas named for them ([`hand_once`]): the photo's encoder during a recording
    pub once: Vec<ChnPort>,
}

/// A reader of a pool: its input port and the frames a second it takes ([`crate::compose`] hands it a canvas when
/// that canvas's time has reached its next one). 0 until its rate is known: the pool's own reader gets it with its
/// feed ([`rate`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reader {
    pub port: ChnPort,
    pub fps: u32,
}

/// How a reader added to a pool takes its canvases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reads {
    /// at this many a second, of the canvases the port writes
    At(u32),
    /// only the canvas named for it ([`hand_once`]): the photo's encoder during a recording
    Once,
}

impl Pool {
    /// The canvas a view (or any address inside a canvas) belongs to.
    pub fn which(&self, phys: u64) -> Option<usize> {
        let bytes = self.size.w as u64 * self.rows as u64 * 3 / 2;
        self.canvases.iter().position(|&c| phys >= c && phys < c + bytes)
    }

    /// Canvas `k` as GE's NV12 frame.
    pub fn nv12(&self, k: usize) -> Nv12 {
        let base = self.canvases[k];
        Nv12 { y: base, uv: base + self.size.w as u64 * self.rows as u64, stride: self.size.w, w: self.size.w, h: self.size.h }
    }
}

struct Ctl {
    file: File,
    pools: Vec<Pool>,
}

static CTL: Mutex<Option<Ctl>> = Mutex::new(None);

fn io(e: std::io::Error, what: &str) -> String {
    format!("microcam_canvas {what}: {e}")
}

/// One command to the module; the error is the module's refusal (EBUSY, EINVAL…).
fn cmd(c: &mut Ctl, s: &str) -> std::io::Result<()> {
    c.file.write_all(s.as_bytes())
}

fn with<T>(f: impl FnOnce(&mut Ctl) -> std::result::Result<T, String>) -> std::result::Result<T, String> {
    let mut g = CTL.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        let file = std::fs::OpenOptions::new().read(true).write(true).open(PATH).map_err(|e| io(e, "open"))?;
        *g = Some(Ctl { file, pools: Vec::new() });
    }
    f(g.as_mut().unwrap())
}

/// For `reader` of the open pool `id`, the module's «after-release»: the hand-overs' times after the reader's last
/// release summed in µs, their count, and «released», the last release in ns.
pub fn after_release(id: u32, reader: ChnPort) -> std::result::Result<(u64, u64, u64), String> {
    let mut s = String::new();
    File::open(PATH).and_then(|mut f| f.read_to_string(&mut s)).map_err(|e| io(e, "status"))?;
    let me = format!("reader {} {}/{}/{} ", module_id(reader.module), reader.dev, reader.chn, reader.port);
    let mut ours = false;
    for l in s.lines() {
        if let Some(rest) = l.strip_prefix("pool ") {
            let w: Vec<&str> = rest.split_whitespace().collect();
            ours = w.first() == Some(&id.to_string().as_str()) && w.contains(&"open");
        } else if ours && l.trim_start().starts_with(&me) {
            let field = |name: &str| l.split_whitespace().skip_while(|w| *w != name).nth(1).ok_or(format!("no {name} in the status"));
            let (sum, n) = field("after-release")?.split_once('/').ok_or("bad after-release")?;
            let bad = |_| "bad after-release".to_string();
            return Ok((sum.parse().map_err(bad)?, n.parse().map_err(bad)?, field("released")?.parse().map_err(bad)?));
        }
    }
    Err(format!("no reader {} of pool {id}", me.trim()))
}

/// The canvases of pool `id` as the module lists them.
fn read_canvases(id: u32) -> std::result::Result<Vec<u64>, String> {
    let mut s = String::new();
    File::open(PATH).and_then(|mut f| f.read_to_string(&mut s)).map_err(|e| io(e, "status"))?;
    let mut out = Vec::new();
    let mut ours = false;
    for l in s.lines() {
        if let Some(rest) = l.strip_prefix("pool ") {
            // the open pool only: one taken down under this id is listed until DISP lets its last canvas go
            let w: Vec<&str> = rest.split_whitespace().collect();
            ours = w.first() == Some(&id.to_string().as_str()) && w.contains(&"open");
        } else if ours {
            let mut w = l.split_whitespace();
            if w.next() == Some("canvas") {
                let k: usize = w.next().and_then(|k| k.parse().ok()).ok_or("bad status")?;
                let phys = w.nth(1).and_then(|p| u64::from_str_radix(p, 16).ok()).ok_or("bad status")?;
                if !l.contains("retired") {
                    if out.len() <= k {
                        out.resize(k + 1, 0);
                    }
                    out[k] = phys;
                }
            }
        }
    }
    Ok(out.into_iter().filter(|&p| p != 0).collect())
}

/// Pool `id` of `n` canvases `size` (`rows` allocated) for the frames of `src` going to `readers`. An encoder among them is
/// put on with the pool itself (the module's tested form); the others are added after it, and a pool one of them
/// refuses is closed again.
pub fn up(id: u32, size: Size, rows: u32, n: u32, src: ChnPort, readers: &[ChnPort]) -> std::result::Result<Pool, String> {
    let first = readers.iter().position(|r| r.module == Module::Venc);
    with(|c| {
        let mut s = format!("pool {id} {} {} {rows} {n} {} {} {}", size.w, size.h, src.dev, src.chn, src.port);
        if let Some(v) = first.map(|i| readers[i]) {
            s += &format!(" {} {}", v.dev, v.chn);
        }
        cmd(c, &s).map_err(|e| io(e, &s))?;
        let mut added = first.map(|i| vec![readers[i]]).unwrap_or_default();
        for (i, r) in readers.iter().enumerate().filter(|(i, _)| Some(*i) != first) {
            if let Err(e) = add_reader(c, id, *r) {
                if let Err(e2) = cmd(c, &format!("close {id}")) {
                    log::error!("microcam_canvas: pool {id} not closed after its reader {i} failed: {e2}");
                }
                return Err(e);
            }
            added.push(*r);
        }
        let taking = added.into_iter().map(|port| Reader { port, fps: 0 }).collect();
        let pool = Pool { id, size, rows, canvases: read_canvases(id)?, readers: taking, once: Vec::new() };
        c.pools.retain(|p| p.id != id);
        c.pools.push(pool.clone());
        let to: Vec<String> = readers.iter().map(|r| name(*r)).collect();
        log::info!("canvas pool {id}: {n} × {size} (rows {rows}), SCL {}/{}/{} → {}", src.dev, src.chn, src.port, to.join(", "));
        Ok(pool)
    })
}

/// An input port in the log.
fn name(r: ChnPort) -> String {
    match r.module {
        Module::Disp => format!("DISP {}/{}", r.dev, r.chn),
        _ => format!("encoder {}/{}", r.dev, r.chn),
    }
}

fn add_reader(c: &mut Ctl, id: u32, r: ChnPort) -> std::result::Result<(), String> {
    let s = format!("reader {id} {} {} {} {}", module_id(r.module), r.dev, r.chn, r.port);
    cmd(c, &s).map_err(|e| io(e, &s))
}

/// One more reader of pool `id`: its input port gets the module's allocator, and from now on it is handed the
/// canvases its rate takes — or only the canvas named for it ([`hand_once`]). The reader's channel must exist.
pub fn reader(id: u32, r: ChnPort, reads: Reads) -> std::result::Result<(), String> {
    with(|c| {
        add_reader(c, id, r)?;
        let p = c.pools.iter_mut().find(|p| p.id == id).ok_or(format!("no pool {id}"))?;
        match reads {
            Reads::Once => {
                p.once.push(r);
                log::info!("canvas pool {id}: {} reads a canvas when one is named for it", name(r));
            }
            Reads::At(fps) => {
                p.readers.push(Reader { port: r, fps });
                log::info!("canvas pool {id}: {} reads it too, {fps} a second", name(r));
            }
        }
        Ok(())
    })
}

/// Reader `r` of pool `id` takes `fps` canvases a second: the pool's own reader, at its feed's rate.
pub fn rate(id: u32, r: ChnPort, fps: u32) -> std::result::Result<(), String> {
    with(|c| {
        let p = c.pools.iter_mut().find(|p| p.id == id).ok_or(format!("no pool {id}"))?;
        let reader = p.readers.iter_mut().find(|x| x.port == r).ok_or(format!("{} reads no pool {id}", name(r)))?;
        reader.fps = fps;
        Ok(())
    })
}

/// Reader `r` of pool `id` taken off (its allocator off, its channel still there): it takes buffers of its own again.
pub fn unreader(id: u32, r: ChnPort) -> std::result::Result<(), String> {
    with(|c| {
        let s = format!("unreader {id} {} {} {} {}", module_id(r.module), r.dev, r.chn, r.port);
        cmd(c, &s).map_err(|e| io(e, &s))?;
        if let Some(p) = c.pools.iter_mut().find(|p| p.id == id) {
            p.readers.retain(|x| x.port != r);
            p.once.retain(|x| *x != r);
        }
        Ok(())
    })
}

/// Pool `id` to `n` canvases (added, or the last ones retired once nobody has them).
pub fn count(id: u32, n: u32) -> std::result::Result<Pool, String> {
    with(|c| {
        let s = format!("count {id} {n}");
        cmd(c, &s).map_err(|e| io(e, &s))?;
        let mut canvases = read_canvases(id)?;
        canvases.truncate(n as usize);
        let p = c.pools.iter_mut().find(|p| p.id == id).ok_or(format!("no pool {id}"))?;
        p.canvases = canvases;
        Ok(p.clone())
    })
}

/// Pool `id` closed: its allocators off (the port's channel and the readers still there), the canvases freed once
/// nobody has them.
pub fn down(id: u32) -> std::result::Result<(), String> {
    with(|c| {
        c.pools.retain(|p| p.id != id);
        cmd(c, &format!("close {id}")).map_err(|e| io(e, "close"))
    })
}

/// The pool as made.
pub fn pool(id: u32) -> Option<Pool> {
    CTL.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|c| c.pools.iter().find(|p| p.id == id).cloned())
}

/// Where the port's window goes in the canvases (`x` on [`GRID`], `y` even); None: no view is lent (covered).
pub fn view(id: u32, at: Option<(u32, u32)>) -> std::result::Result<(), String> {
    with(|c| {
        let s = match at {
            Some((x, y)) => format!("view {id} {x} {y}"),
            None => format!("view {id} none"),
        };
        cmd(c, &s).map_err(|e| io(e, &s))
    })
}

/// Canvas `k` is microcamd's to draw into; false while the port or an encoder has it.
pub fn take(id: u32, k: usize) -> std::result::Result<bool, String> {
    with(|c| match cmd(c, &format!("take {id} {k}")) {
        Ok(()) => Ok(true),
        Err(e) if e.raw_os_error() == Some(libc::EBUSY) => Ok(false),
        Err(e) => Err(io(e, "take")),
    })
}

pub fn give(id: u32, k: usize) -> std::result::Result<(), String> {
    with(|c| cmd(c, &format!("give {id} {k}")).map_err(|e| io(e, "give")))
}

/// A canvas not handed: the reader that did not take it (None: the module refused to name it).
#[derive(Debug)]
pub struct Refused {
    pub reader: Option<ChnPort>,
    pub error: String,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.reader {
            Some(r) => write!(f, "{}: {}", name(r), self.error),
            None => write!(f, "{}", self.error),
        }
    }
}

/// Canvas `k` of pool `id` to every reader of the pool as the frame of `pts`, whatever its rate (a photo's one canvas):
/// the module's allocator on each reader's input gives it to a plain GetBufPa, and takes it back when that reader lets
/// it go. Every reader is tried; the first refusal is returned. Done under the pool's lock: a reader added or taken off ([`reader`], [`unreader`]) meanwhile
/// waits, so no reader is asked for a canvas its allocator no longer gives — its own buffer would go out unwritten.
pub fn hand(id: u32, k: usize, size: Size, pts: u64) -> std::result::Result<(), Refused> {
    hand_to(id, k, size, pts, |_| true)
}

/// [`hand`] to the readers `which` picks only: those whose rate takes this canvas; DISP handed a canvas before the port
/// writes it, the rest once it is written.
pub fn hand_to(id: u32, k: usize, size: Size, pts: u64, which: impl Fn(ChnPort) -> bool) -> std::result::Result<(), Refused> {
    with(|c| {
        cmd(c, &format!("next {id} {k}")).map_err(|e| io(e, "next"))?;
        let readers = c.pools.iter().find(|p| p.id == id).map(|p| p.readers.clone()).unwrap_or_default();
        let mut r = Ok(());
        let mut skipped = false;
        for to in readers.into_iter().map(|r| r.port) {
            if !which(to) {
                skipped = true;
                continue;
            }
            if let Err(error) = give_to(to, size, pts) {
                if r.is_ok() {
                    r = Err(Refused { reader: Some(to), error: error.to_string() });
                }
            }
        }
        // a reader skipped would keep the canvas named, and the port is never lent a canvas still named to a reader
        if skipped {
            cmd(c, &format!("next {id} -1")).map_err(|e| io(e, "next"))?;
        }
        Ok(r)
    })
    .unwrap_or_else(|error| Err(Refused { reader: None, error }))
}

/// Canvas `k` of pool `id`, handed to the readers already, to `to` as well: a reader taken on for one canvas
/// ([`Reads::Once`]). The canvas is the readers' meanwhile, so microcamd does not write it.
pub fn hand_once(id: u32, k: usize, to: ChnPort, size: Size, pts: u64) -> std::result::Result<(), String> {
    with(|c| {
        if !c.pools.iter().any(|p| p.id == id && p.once.contains(&to)) {
            return Err(format!("{} is no one-canvas reader of pool {id}", name(to)));
        }
        cmd(c, &format!("next {id} {k}")).map_err(|e| io(e, "next"))?;
        give_to(to, size, pts).map_err(|e| e.to_string())
    })
}

/// The module's lendings of views to the ports (`/proc/microcam_canvas_lent`, pollable: readable while lendings came since
/// the last [`Lent::read`]); None with a module that has no such file.
pub struct Lent(File);

/// A pool's newest lending: its number (growing), the canvas, the time (ns, the kernel's monotonic clock).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lending {
    pub pool: u32,
    pub seq: u32,
    pub canvas: usize,
    pub ns: u64,
}

impl Lent {
    pub fn open() -> Option<Lent> {
        File::open("/proc/microcam_canvas_lent").ok().map(Lent)
    }

    pub fn fd(&self) -> i32 {
        use std::os::fd::AsRawFd;
        self.0.as_raw_fd()
    }

    /// Each pool's newest lending.
    pub fn read(&self) -> Vec<Lending> {
        use std::os::unix::fs::FileExt;
        let mut buf = [0u8; 256];
        let n = self.0.read_at(&mut buf, 0).unwrap_or(0);
        String::from_utf8_lossy(&buf[..n])
            .lines()
            .filter_map(|l| {
                let w: Vec<&str> = l.split_whitespace().collect();
                Some(Lending { pool: w.first()?.parse().ok()?, seq: w.get(1)?.parse().ok()?, canvas: w.get(2)?.parse().ok()?, ns: w.get(3)?.parse().ok()? })
            })
            .collect()
    }
}

/// The canvas the module named to `to`'s input port: taken with a plain GetBufPa (the module's allocator answers),
/// handed back filled as the frame of `pts` (canvasdisp.c's `give`, which DISP took too).
fn give_to(to: ChnPort, size: Size, pts: u64) -> Result<()> {
    let mut p = chn_port(to);
    // SAFETY: all-zero is a valid BufConf/BufInfo (plain data; the union's frame view is the one set below).
    let mut conf: ffi::MI_SYS_BufConf_t = unsafe { std::mem::zeroed() };
    conf.eBufType = ffi::MI_SYS_BufDataType_e::E_MI_SYS_BUFDATA_FRAME;
    conf.u64TargetPts = pts;
    // SAFETY: writing the frame view of the union, which the buffer type selects.
    unsafe {
        let f = &mut conf.__bindgen_anon_1.stFrameCfg;
        f.u16Width = size.w as u16;
        f.u16Height = size.h as u16;
        f.eFormat = PIXEL_NV12;
        f.eFrameScanMode = ffi::MI_SYS_FrameScanMode_e::E_MI_SYS_FRAME_SCAN_MODE_PROGRESSIVE;
    }
    let mut info: ffi::MI_SYS_BufInfo_t = unsafe { std::mem::zeroed() };
    let mut h: ffi::MI_SYS_BUF_HANDLE = 0;
    check("MI_SYS_ChnInputPortGetBufPa", unsafe { ffi::MI_SYS_ChnInputPortGetBufPa(&mut p, &mut conf, &mut info, &mut h, 0) })?;
    info.u64Pts = pts;
    check("MI_SYS_ChnInputPortPutBufPa", unsafe { ffi::MI_SYS_ChnInputPortPutBufPa(h, &mut info, 0) })
}
