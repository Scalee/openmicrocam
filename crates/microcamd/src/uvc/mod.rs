//! The UVC gadget adapter: owns the V4L2 node of `g_webcam`'s UVC function, answers the host's class requests
//! (probe/commit against the gadget's format list), and turns STREAMON/STREAMOFF into pipeline messages. Frames move
//! through the [`pump`]. One thread for events; the pump has its own.
//!
//! Kernel facts this follows: open activates the function on the bus; STREAMON arrives with the
//! control request's status delayed until our `VIDIOC_STREAMON`, so it must be answered within a bounded budget.

pub mod ioctl;
pub mod pump;

use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use microcam_core::desired::Epoch;
use microcam_core::formats::{FormatTable, HostStream};
use microcam_core::controls::{By, Change};
use microcam_core::uvc_controls::{self as uc, Query};
use microcam_core::Codec;
use mi_sys::uapi as u;

use crate::config::Usb;
use crate::controls::Controls;
use crate::pipeline::{ask, Msg};

const UVC_EVENT_CONNECT: u32 = u::V4L2_EVENT_PRIVATE_START;
const UVC_EVENT_DISCONNECT: u32 = u::V4L2_EVENT_PRIVATE_START + 1;
const UVC_EVENT_STREAMON: u32 = u::V4L2_EVENT_PRIVATE_START + 2;
const UVC_EVENT_STREAMOFF: u32 = u::V4L2_EVENT_PRIVATE_START + 3;
const UVC_EVENT_SETUP: u32 = u::V4L2_EVENT_PRIVATE_START + 4;
const UVC_EVENT_DATA: u32 = u::V4L2_EVENT_PRIVATE_START + 5;

const UVC_SET_CUR: u8 = 0x01;
const UVC_GET_CUR: u8 = 0x81;
/// How long a GET_CUR of a live control waits for the pipeline to read the auto (it may be in a transition)
const LIVE_WAIT: Duration = Duration::from_millis(300);
const UVC_GET_MIN: u8 = 0x82;
const UVC_GET_MAX: u8 = 0x83;
const UVC_GET_RES: u8 = 0x84;
const UVC_GET_LEN: u8 = 0x85;
const UVC_GET_INFO: u8 = 0x86;
const UVC_GET_DEF: u8 = 0x87;
const UVC_VS_PROBE_CONTROL: u8 = 0x01;
const UVC_VS_COMMIT_CONTROL: u8 = 0x02;
/// The STALL answer of the UVC gadget: a response of length -EL2HLT.
const STALL: i32 = -51;

const fn fourcc(a: u8, b: u8, c: u8, d: u8) -> u32 {
    a as u32 | (b as u32) << 8 | (c as u32) << 16 | (d as u32) << 24
}
const FCC_MJPEG: u32 = fourcc(b'M', b'J', b'P', b'G');
const FCC_H264: u32 = fourcc(b'H', b'2', b'6', b'4');
const FCC_H265: u32 = fourcc(b'H', b'2', b'6', b'5');

pub struct Uvc {
    fd: OwnedFd,
    formats: Arc<FormatTable>,
    cfg: Usb,
    tx: Sender<Msg>,
    probe: u::uvc_streaming_control,
    commit: u::uvc_streaming_control,
    /// a SET_CUR whose data is still to come
    pending: Option<Pending>,
    streaming: Option<Streaming>,
    shared: Arc<pump::Shared>,
    controls: Arc<Controls>,
    /// `bRequestErrorCode` of the last VideoControl request, read by hosts after a STALL
    error_code: u8,
    stalls_logged: u32,
}

#[derive(Clone, Copy)]
enum Pending {
    /// probe or commit (the control selector)
    Streaming(u8),
    Control(uc::Control),
}

struct Streaming {
    #[allow(dead_code)]
    epoch: Epoch,
    stream: HostStream,
    bufs_len: usize,
}

impl Uvc {
    pub fn open(cfg: Usb, formats: Arc<FormatTable>, tx: Sender<Msg>, shared: Arc<pump::Shared>, controls: Arc<Controls>) -> std::io::Result<Self> {
        let f = std::fs::OpenOptions::new().read(true).write(true).custom_flags(libc::O_NONBLOCK).open(&cfg.device)?;
        let fd: OwnedFd = f.into();
        let mut cap: u::v4l2_capability = unsafe { std::mem::zeroed() };
        ioctl::call(fd.as_raw_fd(), ioctl::VIDIOC_QUERYCAP, &mut cap)?;
        let name = String::from_utf8_lossy(&cap.card).trim_end_matches('\0').to_string();
        log::info!("UVC gadget {}: '{}' caps {:#x}", cfg.device.display(), name, cap.capabilities);
        for t in [UVC_EVENT_CONNECT, UVC_EVENT_DISCONNECT, UVC_EVENT_STREAMON, UVC_EVENT_STREAMOFF, UVC_EVENT_SETUP, UVC_EVENT_DATA] {
            let mut sub: u::v4l2_event_subscription = unsafe { std::mem::zeroed() };
            sub.type_ = t;
            ioctl::call(fd.as_raw_fd(), ioctl::VIDIOC_SUBSCRIBE_EVENT, &mut sub)?;
        }
        let mut me = Self {
            fd,
            formats,
            cfg,
            tx,
            probe: unsafe { std::mem::zeroed() },
            commit: unsafe { std::mem::zeroed() },
            pending: None,
            streaming: None,
            shared,
            controls,
            error_code: uc::error::NONE,
            stalls_logged: 0,
        };
        let def = me.default_control();
        me.probe = def;
        me.commit = def;
        Ok(me)
    }

    pub fn spawn(self) {
        thread::Builder::new().name("uvc".into()).spawn(move || self.run()).expect("spawn uvc");
    }

    fn run(mut self) {
        loop {
            let mut pfd = libc::pollfd { fd: self.fd.as_raw_fd(), events: libc::POLLPRI, revents: 0 };
            let r = unsafe { libc::poll(&mut pfd, 1, 1000) };
            if r < 0 {
                let e = std::io::Error::last_os_error();
                if e.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                log::error!("uvc poll: {e}");
                thread::sleep(Duration::from_millis(200));
                continue;
            }
            if r == 0 || pfd.revents & libc::POLLPRI == 0 {
                continue;
            }
            let mut ev: u::v4l2_event = unsafe { std::mem::zeroed() };
            if let Err(e) = ioctl::call(self.fd.as_raw_fd(), ioctl::VIDIOC_DQEVENT, &mut ev) {
                log::warn!("DQEVENT: {e}");
                continue;
            }
            // SAFETY: for UVC event types the kernel stores a `struct uvc_event` in the 64-byte union
            let uev: u::uvc_event = unsafe { std::mem::transmute_copy(&ev.u) };
            match ev.type_ {
                UVC_EVENT_CONNECT => log::info!("UVC: host connected (speed {})", unsafe { uev.__bindgen_anon_1.speed }),
                UVC_EVENT_DISCONNECT => {
                    log::info!("UVC: host disconnected");
                    self.pending = None;
                    // what the host set lasts while it is connected
                    self.controls.host_gone();
                    if self.streaming.is_some() {
                        self.stream_off("disconnect");
                    }
                }
                UVC_EVENT_SETUP => self.on_setup(unsafe { uev.__bindgen_anon_1.req }),
                UVC_EVENT_DATA => self.on_data(unsafe { &uev.__bindgen_anon_1.data }),
                UVC_EVENT_STREAMON => self.stream_on(),
                UVC_EVENT_STREAMOFF => self.stream_off("host"),
                other => log::debug!("UVC event {other:#x} ignored"),
            }
        }
    }

    // ---- class requests ------------------------------------------------------------------------------------

    /// Stalled requests are normal (unsupported controls) but the first ones are worth seeing at info level.
    fn note_stall(&mut self, what: &str) {
        if self.stalls_logged < 30 {
            self.stalls_logged += 1;
            log::info!("UVC stall: {what}");
        } else {
            log::debug!("UVC stall: {what}");
        }
    }

    fn respond(&self, resp: &mut u::uvc_request_data) {
        if let Err(e) = ioctl::call(self.fd.as_raw_fd(), ioctl::UVCIOC_SEND_RESPONSE, resp) {
            log::warn!("SEND_RESPONSE: {e}");
        }
    }

    fn on_setup(&mut self, req: u::usb_ctrlrequest) {
        let mut resp: u::uvc_request_data = unsafe { std::mem::zeroed() };
        resp.length = STALL;
        let class = req.bRequestType & 0x60 == 0x20;
        let interface = (req.wIndex & 0xff) as u8;
        let selector = (req.wValue >> 8) as u8;
        if !class {
            // standard requests are the kernel's; nothing to answer
            return;
        }
        if interface == self.cfg.vs_interface {
            self.streaming_request(req.bRequest, selector, req.wLength, &mut resp);
        } else if interface == self.cfg.vc_interface {
            self.control_request(req.bRequest, (req.wIndex >> 8) as u8, selector, &mut resp);
        } else {
            let m = format!("request {:#x} to interface {} (not VC {} / VS {}) cs {}", req.bRequest, interface, self.cfg.vc_interface, self.cfg.vs_interface, selector);
            self.note_stall(&m);
        }
        self.respond(&mut resp);
    }

    fn streaming_request(&mut self, request: u8, cs: u8, wlength: u16, resp: &mut u::uvc_request_data) {
        if cs != UVC_VS_PROBE_CONTROL && cs != UVC_VS_COMMIT_CONTROL {
            return; // STALL
        }
        let len = std::mem::size_of::<u::uvc_streaming_control>().min(wlength as usize).min(26);
        match request {
            UVC_SET_CUR => {
                self.pending = Some(Pending::Streaming(cs));
                resp.length = len as i32;
            }
            UVC_GET_CUR => {
                let c = if cs == UVC_VS_PROBE_CONTROL { self.probe } else { self.commit };
                write_ctrl(resp, &c, len);
            }
            UVC_GET_MIN | UVC_GET_MAX | UVC_GET_DEF | UVC_GET_RES => {
                let c = self.default_control();
                write_ctrl(resp, &c, len);
            }
            UVC_GET_LEN => {
                resp.data[0] = 26;
                resp.data[1] = 0;
                resp.length = 2;
            }
            UVC_GET_INFO => {
                resp.data[0] = 0x03;
                resp.length = 1;
            }
            _ => {}
        }
    }

    /// A VideoControl request: the request error code, or a camera-terminal / processing-unit control. Anything
    /// else (extension units, controls the camera does not have) is left as STALL with its error code.
    fn control_request(&mut self, request: u8, entity: u8, cs: u8, resp: &mut u::uvc_request_data) {
        if entity == 0 {
            match (cs, request) {
                (uc::CS_REQUEST_ERROR_CODE, UVC_GET_CUR) => {
                    resp.data[0] = self.error_code;
                    resp.length = 1;
                }
                (uc::CS_REQUEST_ERROR_CODE, UVC_GET_INFO) => {
                    resp.data[0] = uc::INFO_GET;
                    resp.length = 1;
                }
                _ => self.error_code = uc::error::INVALID_CONTROL,
            }
            return;
        }
        // a value the running auto sets is read from it for GET_CUR (the host must not see the list's manual one); not
        // read in time, or no picture running, it is not ready
        let read = |name| {
            let live = || ask(&self.tx, Msg::Live, LIVE_WAIT).map_err(|e| log::warn!("UVC {name}: the auto's value not read ({e}): not ready")).ok();
            if request == UVC_GET_CUR { self.controls.read(name, live) } else { self.controls.get(name).map(Ok) }
        };
        let Some((c, e)) = uc::lookup(entity, cs).and_then(|c| Some((c, read(c.name)?))) else {
            self.error_code = if uc::known_entity(entity) { uc::error::INVALID_CONTROL } else { uc::error::INVALID_UNIT };
            let m = format!("VC request {request:#x} entity {entity} cs {cs}");
            self.note_stall(&m);
            return;
        };
        let Ok(e) = e else {
            self.error_code = uc::error::NOT_READY;
            let m = format!("VC GET_CUR {}: the auto's value not read, not ready", c.name);
            self.note_stall(&m);
            return;
        };
        let query = match request {
            UVC_GET_CUR => Some(Query::Cur),
            UVC_GET_MIN => Some(Query::Min),
            UVC_GET_MAX => Some(Query::Max),
            UVC_GET_RES => Some(Query::Res),
            UVC_GET_DEF => Some(Query::Def),
            _ => None,
        };
        self.error_code = uc::error::NONE;
        match request {
            UVC_SET_CUR => {
                self.pending = Some(Pending::Control(c));
                resp.length = c.len as i32;
            }
            UVC_GET_INFO => {
                resp.data[0] = c.info(&e);
                resp.length = 1;
            }
            UVC_GET_LEN => {
                resp.data[..2].copy_from_slice(&(c.len as u16).to_le_bytes());
                resp.length = 2;
            }
            _ => match query.and_then(|q| c.answer(q, &e)) {
                Some(v) => resp.length = c.encode(v, &mut resp.data) as i32,
                None => self.error_code = uc::error::INVALID_REQUEST,
            },
        }
    }

    fn on_data(&mut self, data: &u::uvc_request_data) {
        let cs = match self.pending.take() {
            None => return,
            Some(Pending::Control(c)) => {
                let n = (data.length.max(0) as usize).min(data.data.len());
                match self.controls.get(c.name).and_then(|e| c.decode(&data.data[..n], &e)) {
                    Some(v) => {
                        let name = c.name;
                        self.controls.change(name, Change::Set(v), By::Host, Box::new(move |r| match r {
                            Ok(v) => log::info!("UVC control: {name} = {v}"),
                            Err(r) => log::warn!("UVC control {name}: {}", r.error),
                        }));
                    }
                    None => log::warn!("UVC control {}: value {:?} not accepted", c.name, &data.data[..n]),
                }
                return;
            }
            Some(Pending::Streaming(cs)) => cs,
        };
        let n = (data.length.max(0) as usize).min(26);
        if n < 8 {
            log::warn!("short probe/commit data ({n} bytes)");
            return;
        }
        let mut ctrl: u::uvc_streaming_control = unsafe { std::mem::zeroed() };
        // SAFETY: both are plain byte layouts; we copy at most 26 bytes into a 34-byte struct
        unsafe { std::ptr::copy_nonoverlapping(data.data.as_ptr(), &mut ctrl as *mut _ as *mut u8, n) };
        let mut negotiated = self.negotiate(&ctrl);
        negotiated.dwFrameInterval = self.deliverable(&negotiated);
        if cs == UVC_VS_PROBE_CONTROL {
            self.probe = negotiated;
        } else {
            self.commit = negotiated;
            let (fi, fr, iv) = (negotiated.bFormatIndex, negotiated.bFrameIndex, negotiated.dwFrameInterval);
            log::info!("UVC commit: format {fi} frame {fr} interval {iv} ({} fps)", if iv > 0 { 10_000_000 / iv } else { 0 });
        }
    }

    /// Clamp a host proposal to the table and fill in what the camera decides.
    fn negotiate(&self, req: &u::uvc_streaming_control) -> u::uvc_streaming_control {
        let fi = if self.formats.format(req.bFormatIndex).is_ok() { req.bFormatIndex } else { 1 };
        let fmt = self.formats.format(fi).unwrap();
        let fr = if self.formats.frame(fi, req.bFrameIndex).is_ok() { req.bFrameIndex } else { fmt.frames.first().map(|f| f.index).unwrap_or(1) };
        let frame = self.formats.frame(fi, fr).unwrap();
        let interval = frame.match_interval(req.dwFrameInterval);
        let mut c: u::uvc_streaming_control = unsafe { std::mem::zeroed() };
        c.bmHint = 1;
        c.bFormatIndex = fi;
        c.bFrameIndex = fr;
        c.dwFrameInterval = interval;
        c.dwMaxVideoFrameSize = max_frame_size(fmt.codec, frame.width, frame.height);
        c.dwMaxPayloadTransferSize = self.cfg.max_payload;
        c.dwClockFrequency = 48_000_000;
        c.bmFramingInfo = 3;
        c.bPreferedVersion = 1;
        c.bMinVersion = 1;
        c.bMaxVersion = 1;
        c
    }

    /// The interval of the rate the camera will deliver for this proposal now (the policy caps it by the sensor mode the
    /// monitor class allows: 1080p at 40 fps beside a 4K monitor). A host times its frame wait by the negotiated rate (OBS:
    /// two frame periods, then it resets the stream), so the answer must not promise more than arrives.
    fn deliverable(&self, c: &u::uvc_streaming_control) -> u32 {
        let iv = c.dwFrameInterval;
        let Ok(host) = self.formats.resolve(c.bFormatIndex, c.bFrameIndex, iv) else { return iv };
        match ask(&self.tx, |r| Msg::UsbRate(host, r), Duration::from_secs(3)) {
            Ok(Some(fps)) if fps > 0 && fps < host.fps => {
                log::info!("UVC probe: {:?} {} asked @{}, delivered now @{fps}", host.codec, host.size, host.fps);
                (10_000_000 + fps / 2) / fps
            }
            Ok(_) => iv,
            Err(e) => {
                log::warn!("UVC probe: rate unknown ({e}); answering @{}", host.fps);
                iv
            }
        }
    }

    fn default_control(&self) -> u::uvc_streaming_control {
        let mut req: u::uvc_streaming_control = unsafe { std::mem::zeroed() };
        (req.bFormatIndex, req.bFrameIndex) = self.formats.default();
        self.negotiate(&req)
    }

    // ---- streaming -----------------------------------------------------------------------------------------

    fn stream_on(&mut self) {
        let t0 = Instant::now();
        let budget = Duration::from_millis(self.cfg.streamon_budget_ms);
        if self.streaming.is_some() {
            log::warn!("STREAMON while streaming: restarting");
            self.stream_off("restart");
        }
        let c = self.commit;
        let host = match self.formats.resolve(c.bFormatIndex, c.bFrameIndex, c.dwFrameInterval) {
            Ok(h) => h,
            Err(e) => {
                log::error!("STREAMON with an unresolvable commit: {e}");
                self.answer_unserved(None);
                return;
            }
        };
        log::info!("STREAMON: {:?} {} @{}", host.codec, host.size, host.fps);
        // 1. the pipeline builds the stream (sensor mode, encoder); bounded wait
        let epoch = match ask(&self.tx, |r| Msg::StreamOn(host, r), budget) {
            Ok(Ok(e)) => e,
            Ok(Err(e)) | Err(e) => {
                log::error!("STREAMON refused: {e}");
                self.answer_unserved(None);
                // the pipeline keeps the host's stream it could not build (or is still building): taken back
                self.abort_stream();
                return;
            }
        };
        // 2. gadget buffers
        let fd = self.fd.as_raw_fd();
        let size = max_frame_size(host.codec, host.size.w, host.size.h);
        if let Err(e) = set_format(fd, host, size) {
            log::error!("S_FMT: {e}");
            self.answer_unserved(None);
            self.abort_stream();
            return;
        }
        let bufs = match request_buffers(fd, self.cfg.buffers) {
            Ok(b) => b,
            Err(e) => {
                log::error!("REQBUFS/mmap: {e}");
                self.answer_unserved(None);
                self.abort_stream();
                return;
            }
        };
        // 3. the first frame before STREAMON, so the host never sees an empty payload (the vendor's did)
        let enc = {
            let s = self.shared.st.lock().unwrap();
            s.encoder
        };
        let Some(enc) = enc else {
            log::error!("pipeline reported ready but no encoder is registered");
            self.answer_unserved(Some(bufs));
            self.abort_stream();
            return;
        };
        // the encoder kept running while nobody streamed: what it holds is old. Drain it, then (H.26x) ask for an IDR,
        // and start the host on a fresh frame it can decode on its own.
        let mut stale = 0;
        let mut released = Ok(());
        while let Ok(Some(st)) = self.shared.take_frame(&enc, 0) {
            released = released.and(self.shared.release_frame(&enc, st));
            stale += 1;
        }
        if stale > 0 {
            log::info!("dropped {stale} frames the encoder held from before this stream");
        }
        if enc.codec != Codec::Mjpeg {
            if let Err(e) = mi::venc::request_idr(enc.dev, enc.chn) {
                log::warn!("IDR request: {e}");
            }
        }
        let mut first = None;
        while t0.elapsed() < budget {
            match self.shared.take_frame(&enc, 40) {
                Ok(Some(st)) if !st.keyframe => {
                    released = released.and(self.shared.release_frame(&enc, st));
                }
                Ok(Some(st)) => {
                    first = Some(st);
                    break;
                }
                Ok(None) => continue,
                Err(e) => {
                    log::warn!("first frame: {e}");
                    thread::sleep(Duration::from_millis(20));
                }
            }
        }
        let Some(first) = first else {
            log::error!("no frame from the encoder within the STREAMON budget");
            self.answer_unserved(Some(bufs));
            self.abort_stream();
            return;
        };
        let b0 = &bufs[0];
        let b0_index = b0.index;
        let n = if first.len() <= b0.len { first.copy_to(unsafe { std::slice::from_raw_parts_mut(b0.ptr, b0.len) }) } else { 0 };
        released = released.and(self.shared.release_frame(&enc, first));
        if released.is_err() {
            // the encoder cannot be stopped cleanly now (the pump's stop says so): no stream on it
            log::error!("a frame could not be given back to the encoder");
            self.answer_unserved(Some(bufs));
            self.abort_stream();
            return;
        }
        if n == 0 || pump::queue(fd, b0.index, n as u32).is_err() {
            log::error!("could not queue the first frame");
            self.answer_unserved(Some(bufs));
            self.abort_stream();
            return;
        }
        // the other buffers were never queued: the pump takes them from `free` first, then lives off DQBUF
        let bufs_len = bufs.len();
        let free: Vec<u32> = bufs.iter().map(|b| b.index).filter(|&i| i != b0_index).collect();
        self.shared.set_gadget(Some(pump::Gadget { fd, bufs, free }));
        if let Err(e) = ioctl::call_int(fd, ioctl::VIDIOC_STREAMON, u::v4l2_buf_type::V4L2_BUF_TYPE_VIDEO_OUTPUT as i32) {
            log::error!("VIDIOC_STREAMON: {e}");
            if let Some(g) = self.shared.take_gadget() {
                release_buffers(fd, g.bufs);
            }
            self.abort_stream();
            return;
        }
        log::info!("stream running after {} ms (epoch {})", t0.elapsed().as_millis(), epoch.0);
        self.streaming = Some(Streaming { epoch, stream: host, bufs_len });
    }

    /// A stream start that cannot be served is answered all the same. The host's request that selected the streaming
    /// interface has its status held back by the gadget until our `VIDIOC_STREAMON`; left unanswered it times out on
    /// the host and the gadget's count of held-back statuses stays one up, after which no later start is answered
    /// either until the camera is enumerated again. So the gadget's
    /// stream is started without a frame and stopped at once — through `bufs` when the start got as far as having
    /// them: the host gets its status and a stream that brings nothing.
    fn answer_unserved(&mut self, bufs: Option<Vec<pump::Buf>>) {
        let fd = self.fd.as_raw_fd();
        let bufs = match bufs.map_or_else(|| request_buffers(fd, 1), Ok) {
            Ok(b) => b,
            Err(e) => {
                log::error!("STREAMON left unanswered (REQBUFS: {e}): the host's request times out, and later starts may until the camera is plugged in again");
                return;
            }
        };
        let kind = u::v4l2_buf_type::V4L2_BUF_TYPE_VIDEO_OUTPUT as i32;
        match ioctl::call_int(fd, ioctl::VIDIOC_STREAMON, kind) {
            Ok(()) => {
                if let Err(e) = ioctl::call_int(fd, ioctl::VIDIOC_STREAMOFF, kind) {
                    log::warn!("VIDIOC_STREAMOFF: {e}");
                }
                log::warn!("STREAMON answered with a stream that brings no frames");
            }
            Err(e) => log::error!("STREAMON left unanswered (VIDIOC_STREAMON: {e}): the host's request times out, and later starts may until the camera is plugged in again"),
        }
        release_buffers(fd, bufs);
    }

    /// The pipeline built a stream we could not deliver: take it down again.
    fn abort_stream(&mut self) {
        let _ = ask(&self.tx, Msg::StreamOff, Duration::from_secs(10));
    }

    fn stream_off(&mut self, why: &str) {
        let Some(st) = self.streaming.take() else {
            // a STREAMOFF without a stream: still tell the pipeline, a host stream might be registered
            let _ = ask(&self.tx, Msg::StreamOff, Duration::from_secs(10));
            return;
        };
        log::info!("STREAMOFF ({why}): {:?} {} @{}", st.stream.codec, st.stream.size, st.stream.fps);
        let fd = self.fd.as_raw_fd();
        // quiescence: the pump steps away from the gadget and the encoder before anything is torn down
        let gadget = self.shared.take_gadget();
        if !self.shared.wait_quiescent(Duration::from_millis(800)) {
            log::error!("pump did not go quiescent; continuing with STREAMOFF");
        }
        if let Err(e) = ioctl::call_int(fd, ioctl::VIDIOC_STREAMOFF, u::v4l2_buf_type::V4L2_BUF_TYPE_VIDEO_OUTPUT as i32) {
            log::warn!("VIDIOC_STREAMOFF: {e}");
        }
        if let Some(g) = gadget {
            release_buffers(fd, g.bufs);
        }
        let _ = st.bufs_len;
        let _ = ask(&self.tx, Msg::StreamOff, Duration::from_secs(10));
    }
}

fn write_ctrl(resp: &mut u::uvc_request_data, c: &u::uvc_streaming_control, len: usize) {
    // SAFETY: copying `len` ≤ 26 bytes of a packed plain struct into the 60-byte data array
    unsafe { std::ptr::copy_nonoverlapping(c as *const _ as *const u8, resp.data.as_mut_ptr(), len) };
    resp.length = len as i32;
}

/// Buffer size per frame. The vendor used w×h for every codec; MJPEG at quality 15 is ~0.1 B/px and an H.26x IDR a
/// fraction of that, so w×h/2 and w×h/4 (≥ 512 KiB) keep three buffers of 4K under 13 MB of RAM.
fn max_frame_size(codec: Codec, w: u32, h: u32) -> u32 {
    match codec {
        Codec::Mjpeg => (w * h / 2).max(256 * 1024),
        Codec::H264 | Codec::H265 => (w * h / 4).max(512 * 1024),
    }
}

fn set_format(fd: i32, host: HostStream, sizeimage: u32) -> std::io::Result<()> {
    let mut f: u::v4l2_format = unsafe { std::mem::zeroed() };
    f.type_ = u::v4l2_buf_type::V4L2_BUF_TYPE_VIDEO_OUTPUT;
    let pix = unsafe { &mut f.fmt.pix };
    pix.width = host.size.w;
    pix.height = host.size.h;
    pix.pixelformat = match host.codec {
        Codec::Mjpeg => FCC_MJPEG,
        Codec::H264 => FCC_H264,
        Codec::H265 => FCC_H265,
    };
    pix.field = u::v4l2_field::V4L2_FIELD_NONE;
    pix.sizeimage = sizeimage;
    ioctl::call(fd, ioctl::VIDIOC_S_FMT, &mut f)
}

fn request_buffers(fd: i32, count: u32) -> std::io::Result<Vec<pump::Buf>> {
    let mut rb: u::v4l2_requestbuffers = unsafe { std::mem::zeroed() };
    rb.count = count;
    rb.type_ = u::v4l2_buf_type::V4L2_BUF_TYPE_VIDEO_OUTPUT;
    rb.memory = u::v4l2_memory::V4L2_MEMORY_MMAP;
    ioctl::call(fd, ioctl::VIDIOC_REQBUFS, &mut rb)?;
    let mut bufs = Vec::new();
    for i in 0..rb.count {
        let mut b: u::v4l2_buffer = unsafe { std::mem::zeroed() };
        b.type_ = u::v4l2_buf_type::V4L2_BUF_TYPE_VIDEO_OUTPUT;
        b.memory = u::v4l2_memory::V4L2_MEMORY_MMAP;
        b.index = i;
        if let Err(e) = ioctl::call(fd, ioctl::VIDIOC_QUERYBUF, &mut b) {
            release_buffers(fd, bufs);
            return Err(e);
        }
        let len = b.length as usize;
        let off = unsafe { b.m.offset } as libc::off_t;
        // SAFETY: mapping a V4L2 buffer the driver just described.
        let p = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, off) };
        if p == libc::MAP_FAILED {
            let e = std::io::Error::last_os_error();
            release_buffers(fd, bufs);
            return Err(e);
        }
        bufs.push(pump::Buf { index: i, ptr: p as *mut u8, len });
    }
    Ok(bufs)
}

fn release_buffers(fd: i32, bufs: Vec<pump::Buf>) {
    for b in bufs {
        unsafe { libc::munmap(b.ptr as *mut _, b.len) };
    }
    let mut rb: u::v4l2_requestbuffers = unsafe { std::mem::zeroed() };
    rb.count = 0;
    rb.type_ = u::v4l2_buf_type::V4L2_BUF_TYPE_VIDEO_OUTPUT;
    rb.memory = u::v4l2_memory::V4L2_MEMORY_MMAP;
    let _ = ioctl::call(fd, ioctl::VIDIOC_REQBUFS, &mut rb);
}
