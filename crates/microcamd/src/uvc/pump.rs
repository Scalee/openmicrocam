//! The pump: encoder output → gadget buffers. One thread, started once; it runs only while both the pipeline has
//! declared an encoder for a stream session (`PumpControl::start`) and the gadget side has buffers queued
//! (after `VIDIOC_STREAMON`). Stopping is a barrier: the pipeline's `PumpControl::stop` returns only after the pump
//! has released every encoder stream it held and stepped away from the channel (ARCHITECTURE §3).

use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use microcam_core::desired::Epoch;
use microcam_core::Codec;
use mi::hal::PumpControl;
use mi_sys::uapi as u;

use super::ioctl;

/// A mapped V4L2 output buffer of the gadget.
pub struct Buf {
    pub index: u32,
    pub ptr: *mut u8,
    pub len: usize,
}

unsafe impl Send for Buf {}

pub struct Gadget {
    pub fd: i32,
    pub bufs: Vec<Buf>,
    /// buffers never queued yet (DQBUF only returns queued ones)
    pub free: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Encoder {
    pub epoch: Epoch,
    pub dev: u32,
    pub chn: u32,
    pub codec: Codec,
}

#[derive(Default)]
pub struct State {
    pub encoder: Option<Encoder>,
    pub gadget: Option<Gadget>,
    /// set by the pump whenever it is idle (no stream held, not inside an MI call)
    pub quiescent: bool,
    /// a frame of this session could not be given back to the encoder: the channel cannot stop cleanly
    pub release_failed: bool,
    /// frames queued to the gadget, dropped, too big for a buffer: counted for microcamd's life, never reset
    pub frames: u64,
    pub dropped: u64,
    pub oversize: u64,
    /// the three counts when this stream's encoder session started
    pub session: (u64, u64, u64),
    pub last_frame: Option<Instant>,
    /// capture → handed to the gadget
    pub delay: Delay,
}

impl State {
    /// Frames queued, dropped and too big since this stream's encoder session started.
    pub fn since_start(&self) -> (u64, u64, u64) {
        let (f, d, o) = self.session;
        (self.frames - f, self.dropped - d, self.oversize - o)
    }
}

/// How long frames take from the sensor to the gadget's queue, in windows of [`DELAY_WINDOW`] (the media clock that
/// stamps a frame at capture, read again when it is queued).
#[derive(Default)]
pub struct Delay {
    /// the last complete window: average and largest, in ms
    pub last: Option<(f32, f32)>,
    since: Option<Instant>,
    frames: u32,
    sum_us: u64,
    max_us: u64,
}

const DELAY_WINDOW: Duration = Duration::from_secs(10);

impl Delay {
    fn add(&mut self, us: u64) {
        let since = *self.since.get_or_insert_with(Instant::now);
        self.frames += 1;
        self.sum_us += us;
        self.max_us = self.max_us.max(us);
        if since.elapsed() >= DELAY_WINDOW {
            let last = Some((self.sum_us as f32 / self.frames as f32 / 1000.0, self.max_us as f32 / 1000.0));
            *self = Delay { last, ..Delay::default() };
        }
    }
}

pub struct Shared {
    pub st: Mutex<State>,
    pub cv: Condvar,
}

impl Shared {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { st: Mutex::new(State { quiescent: true, ..Default::default() }), cv: Condvar::new() })
    }

    /// Wait until the pump reports idle; false on timeout.
    pub fn wait_quiescent(&self, timeout: Duration) -> bool {
        let g = self.st.lock().unwrap();
        let (g, r) = self.cv.wait_timeout_while(g, timeout, |s| !s.quiescent).unwrap();
        drop(g);
        !r.timed_out()
    }

    /// An encoded frame of `enc`, waited for up to `timeout_ms`.
    pub fn take_frame(&self, enc: &Encoder, timeout_ms: i32) -> mi::Result<Option<mi::venc::Stream>> {
        mi::venc::get_stream(enc.dev, enc.chn, enc.codec, timeout_ms)
    }

    /// A frame taken with [`Shared::take_frame`] back to the encoder. A failed release leaves the channel unable to
    /// stop (`mi::venc::get_stream`), so it sticks and the pump's stop reports it.
    pub fn release_frame(&self, enc: &Encoder, st: mi::venc::Stream) -> mi::Result<()> {
        let r = mi::venc::release(enc.dev, enc.chn, st);
        if let Err(e) = &r {
            log::error!("usb: {e}");
            self.st.lock().unwrap().release_failed = true;
        }
        r
    }

    pub fn set_gadget(&self, g: Option<Gadget>) {
        let mut s = self.st.lock().unwrap();
        s.gadget = g;
        self.cv.notify_all();
    }

    pub fn take_gadget(&self) -> Option<Gadget> {
        let mut s = self.st.lock().unwrap();
        let g = s.gadget.take();
        self.cv.notify_all();
        g
    }
}

/// The pipeline's handle (lives in `MiHal`).
pub struct Handle(pub Arc<Shared>);

impl PumpControl for Handle {
    fn start(&mut self, epoch: Epoch, dev: u32, chn: u32, codec: Codec) -> Result<(), String> {
        let mut s = self.0.st.lock().unwrap();
        s.encoder = Some(Encoder { epoch, dev, chn, codec });
        s.session = (s.frames, s.dropped, s.oversize);
        s.release_failed = false;
        s.delay = Delay::default();
        self.0.cv.notify_all();
        Ok(())
    }

    fn stop(&mut self) -> Result<(), String> {
        {
            let mut s = self.0.st.lock().unwrap();
            s.encoder = None;
            self.0.cv.notify_all();
        }
        if !self.0.wait_quiescent(Duration::from_millis(800)) {
            return Err("pump did not reach quiescence in 800 ms".into());
        }
        if self.0.st.lock().unwrap().release_failed {
            return Err("a frame could not be given back to the encoder".into());
        }
        Ok(())
    }
}

/// Spawn the pump thread.
pub fn spawn(shared: Arc<Shared>) {
    thread::Builder::new().name("pump".into()).spawn(move || run(shared)).expect("spawn pump");
}

fn run(shared: Arc<Shared>) {
    // H.26x: after a frame the host never got, everything up to the next IDR references it — skip to an IDR
    let mut need_key: Option<Encoder> = None;
    loop {
        // wait for work
        let (enc, fd, bufs_len) = {
            let g = shared.st.lock().unwrap();
            let mut g = shared.cv.wait_while(g, |s| s.encoder.is_none() || s.gadget.is_none()).unwrap();
            g.quiescent = false;
            let enc = g.encoder.unwrap();
            let gd = g.gadget.as_ref().unwrap();
            (enc, gd.fd, gd.bufs.len())
        };
        if bufs_len > 0 {
            pump_one(&shared, enc, fd, &mut need_key);
        }
        idle(&shared);
    }
}

/// One encoded frame. It is taken out of the encoder as soon as it is there, whether the gadget has room or not, so a
/// lagging host does not fill the encoder's output ring, where a frame that finds no room is dropped unseen (the USB
/// compositor waits for each frame it put in, `mi::compose`). It goes into the first gadget buffer that is free; when a newer frame is ready
/// first, this one is lost and the newer one goes instead — the host always gets the freshest frame.
fn pump_one(shared: &Shared, enc: Encoder, fd: i32, need_key: &mut Option<Encoder>) {
    let st = match shared.take_frame(&enc, 40) {
        Ok(Some(st)) => st,
        Ok(None) => return,
        Err(e) => {
            log::warn!("pump: {e}");
            thread::sleep(Duration::from_millis(20));
            return;
        }
    };
    if *need_key == Some(enc) && !st.keyframe {
        release(shared, &enc, st);
        return;
    }
    // a gadget buffer: one never queued, one handed back already, or the first one back before a newer frame is ready
    let idx = loop {
        let free = {
            let mut s = shared.st.lock().unwrap();
            if s.encoder != Some(enc) {
                drop(s);
                release(shared, &enc, st);
                return;
            }
            s.gadget.as_mut().and_then(|g| g.free.pop())
        };
        if let Some(i) = free.or_else(|| dequeue(fd, Duration::ZERO)) {
            break i;
        }
        let newer = mi::venc::waiting(enc.dev, enc.chn).unwrap_or_else(|e| {
            log::warn!("pump: {e}; the frame is let go");
            1
        });
        if newer > 0 {
            release(shared, &enc, st);
            shared.st.lock().unwrap().dropped += 1;
            *need_key = lost_frame(&enc);
            return;
        }
        if let Some(i) = dequeue(fd, Duration::from_millis(2)) {
            break i;
        }
    };
    // still the same session?
    let (ptr, len) = {
        let s = shared.st.lock().unwrap();
        match (&s.encoder, &s.gadget) {
            (Some(e), Some(g)) if *e == enc => match g.bufs.iter().find(|b| b.index == idx) {
                Some(b) => (b.ptr, b.len),
                None => {
                    drop(s);
                    release(shared, &enc, st);
                    return;
                }
            },
            _ => {
                drop(s);
                release(shared, &enc, st);
                return;
            }
        }
    };
    if *need_key == Some(enc) {
        *need_key = None;
    }
    let (n, pts) = (st.len(), st.pts());
    let written = if n <= len {
        // SAFETY: the buffer is our mmap of the gadget's queue, `len` bytes, not queued right now.
        let dst = unsafe { std::slice::from_raw_parts_mut(ptr, len) };
        st.copy_to(dst)
    } else {
        0
    };
    release(shared, &enc, st);
    let mut s = shared.st.lock().unwrap();
    if written == 0 {
        s.oversize += 1;
        // the buffer stays ours for the next frame
        if let Some(g) = s.gadget.as_mut() {
            g.free.push(idx);
        }
        drop(s);
        *need_key = lost_frame(&enc);
        return;
    }
    if queue(fd, idx, written as u32).is_err() {
        s.dropped += 1;
        drop(s);
        *need_key = lost_frame(&enc);
    } else {
        s.frames += 1;
        s.last_frame = Some(Instant::now());
        s.delay.add(mi::sys::now_pts().saturating_sub(pts));
    }
}

fn release(shared: &Shared, enc: &Encoder, st: mi::venc::Stream) {
    let _ = shared.release_frame(enc, st);
}

/// A frame of this session will never reach the host. MJPEG frames stand alone; an H.26x stream needs a new IDR.
fn lost_frame(enc: &Encoder) -> Option<Encoder> {
    if enc.codec == Codec::Mjpeg {
        return None;
    }
    if let Err(e) = mi::venc::request_idr(enc.dev, enc.chn) {
        log::warn!("pump: IDR request after a lost frame: {e}");
    }
    Some(*enc)
}

fn idle(shared: &Arc<Shared>) {
    let mut s = shared.st.lock().unwrap();
    s.quiescent = true;
    shared.cv.notify_all();
}

/// Wait for the gadget to hand back a buffer (POLLOUT), then DQBUF.
fn dequeue(fd: i32, timeout: Duration) -> Option<u32> {
    let mut pfd = libc::pollfd { fd, events: libc::POLLOUT, revents: 0 };
    // SAFETY: one pollfd, valid for the call.
    let r = unsafe { libc::poll(&mut pfd, 1, timeout.as_millis() as i32) };
    if r <= 0 || pfd.revents & libc::POLLOUT == 0 {
        return None;
    }
    let mut b: u::v4l2_buffer = unsafe { std::mem::zeroed() };
    b.type_ = u::v4l2_buf_type::V4L2_BUF_TYPE_VIDEO_OUTPUT;
    b.memory = u::v4l2_memory::V4L2_MEMORY_MMAP;
    ioctl::call(fd, ioctl::VIDIOC_DQBUF, &mut b).ok()?;
    Some(b.index)
}

pub fn queue(fd: i32, index: u32, bytesused: u32) -> std::io::Result<()> {
    let mut b: u::v4l2_buffer = unsafe { std::mem::zeroed() };
    b.type_ = u::v4l2_buf_type::V4L2_BUF_TYPE_VIDEO_OUTPUT;
    b.memory = u::v4l2_memory::V4L2_MEMORY_MMAP;
    b.index = index;
    b.bytesused = bytesused;
    ioctl::call(fd, ioctl::VIDIOC_QBUF, &mut b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_session_keeps_the_life_counts_and_starts_its_own() {
        let shared = Shared::new();
        let mut h = Handle(shared.clone());
        h.start(Epoch(1), 0, 0, Codec::Mjpeg).unwrap();
        shared.st.lock().unwrap().frames += 450;
        shared.st.lock().unwrap().dropped += 2;
        // a transition restarts the encoder session: the health line's rate (life counts) goes on, the status's starts at 0
        h.start(Epoch(2), 0, 0, Codec::Mjpeg).unwrap();
        shared.st.lock().unwrap().frames += 60;
        let s = shared.st.lock().unwrap();
        assert_eq!((s.frames, s.dropped), (510, 2));
        assert_eq!(s.since_start(), (60, 0, 0));
    }
}
