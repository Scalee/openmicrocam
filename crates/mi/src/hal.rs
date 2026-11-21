//! `MiHal`: executes the reconciler's operations against the real hardware. One instance, owned by the pipeline
//! thread; the USB pump is reached through [`PumpControl`] because starting/stopping it is an operation in the same
//! ordered list as the MI calls (the quiescence barrier sits before any encoder change).

use microcam_core::reconcile::topo;
use microcam_core::reconcile::{ChnPort, FeedWay, Hal, HalError, Op};
use microcam_core::{Rect, Size};

use crate::{compose, disp, feed, hdmi, isp, picture, scl, sensor, sys, sysfs, tune, venc, vif, MiError};

/// The daemon's USB pump, as the pipeline sees it.
pub trait PumpControl: Send {
    /// Start delivering the encoder's output for this stream session.
    fn start(&mut self, epoch: microcam_core::desired::Epoch, venc_dev: u32, venc_chn: u32, codec: microcam_core::Codec) -> Result<(), String>;
    /// Stop and wait for the quiescence barrier: last MI call returned, every stream released. Err = barrier timed
    /// out; the caller must then treat the encoder channel as poisoned.
    fn stop(&mut self) -> Result<(), String>;
}

/// The daemon's recorder, as the pipeline sees it.
pub trait RecorderControl: Send {
    /// Start writing the encoder's output into this file.
    fn start(&mut self, venc_dev: u32, venc_chn: u32, codec: microcam_core::Codec, path: &str, size: microcam_core::Size, fps: u32) -> Result<(), String>;
    /// Stop taking frames: returns once the recorder holds no encoder stream and has left the channel (the file is
    /// finished in the background). Err = it did not get there in time; the encoder channel is then poisoned.
    fn stop(&mut self) -> Result<(), String>;
}

/// The daemon's player thread, as the pipeline sees it: it sends the decoder the samples of the file it opened.
pub trait PlayerControl: Send {
    /// Start sending `path` (opened by the daemon beforehand) from its start at its own times.
    fn start(&mut self, path: &str) -> Result<(), String>;
    /// Stop sending: returns once the thread sends no more.
    fn stop(&mut self) -> Result<(), String>;
}

/// The daemon's pictures (the menu's full-colour resources), as the pipeline sees them.
pub trait PictureSource: Send {
    /// a camera picture taken for `id`
    fn grabbed(&self, id: microcam_core::desired::PictureId, frame: picture::Frame);
}

/// Receives the hardware defaults of the picture knobs after every IQ load.
pub type BaselineSink = Box<dyn FnMut(Vec<(microcam_core::tuning::Knob, i32)>) + Send>;

/// A pump that does nothing (HDMI-only runs, dry runs).
pub struct NoPump;

impl PumpControl for NoPump {
    fn start(&mut self, _: microcam_core::desired::Epoch, _: u32, _: u32, _: microcam_core::Codec) -> Result<(), String> {
        Ok(())
    }
    fn stop(&mut self) -> Result<(), String> {
        Ok(())
    }
}

pub struct MiHal {
    pump: Box<dyn PumpControl>,
    recorder: Box<dyn RecorderControl>,
    player: Box<dyn PlayerControl>,
    /// the sensor's plane info, read after every sensor enable (the VIF is configured from it)
    plane: Option<sensor::PlaneInfo>,
    /// pad and max fps of the mode last set: a vendor driver's rate goes through MI_SNR_SetFps, where 0 means this
    snr_mode: Option<(u32, u32)>,
    /// the ISP channel the AE runs on and the frame rate the sensor runs at: the AE's exposure limit follows them
    ae_chn: Option<(u32, u32)>,
    fps_now: u32,
    ae_limit_logged: bool,
    /// the encoder currently feeding USB: (dev, chn, codec)
    pub encoder: Option<(u32, u32, microcam_core::Codec)>,
    hdmi_open: bool,
    /// the IQ file's values of the structures the picture knobs change, read after every IQ load
    baseline: Option<tune::Baseline>,
    /// told the hardware defaults of the knobs whose untouched state lives in the ISP, after every IQ load
    on_baseline: BaselineSink,
    pictures: Box<dyn PictureSource>,
    /// the feed thread: every encoder's frames
    feeder: feed::Feeder,
    /// the encoder channels the feeds found lost since each was last fed, read or built
    lost: feed::Lost,
    /// told when a feed's encoder is lost
    on_lost: feed::OnLost,
    /// the `ui`'s still sets for the compositors, by pool: USB's, the media output's, HDMI's
    patches: [std::sync::Arc<compose::PatchBox>; 3],
    /// the encoder channels an SCL port is bound into in a ring, with that port (`VencUp`'s `bound`): their
    /// `VencDown` takes the bind off between stopping and destroying them
    rings: Vec<(u32, ChnPort)>,
}

impl MiHal {
    /// The AE's longest exposure for the frame rate now: whole flicker half-cycles inside 98 % of the frame (the
    /// sensor leaves a few lines), the frame itself when anti-flicker is off. Logged when it changes.
    fn ae_limit(&mut self) {
        let (Some((dev, chn)), true) = (self.ae_chn, self.fps_now > 0) else { return };
        let frame_us = 1_000_000 / self.fps_now * 98 / 100;
        let max = match self.baseline.as_ref().and_then(tune::flicker_half_us) {
            Some(h) if frame_us >= h => frame_us / h * h,
            _ => frame_us,
        };
        match isp::set_ae_max_shutter(dev, chn, max) {
            Ok(was) => {
                if !self.ae_limit_logged {
                    log::info!("AE exposure limit as read: {:?}", &was[..8]);
                    self.ae_limit_logged = true;
                }
                if was[1] != max {
                    log::info!("AE max shutter {} -> {} us ({} fps)", was[1], max, self.fps_now);
                }
            }
            Err(e) => log::warn!("AE exposure limit: {e}"),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pump: Box<dyn PumpControl>,
        recorder: Box<dyn RecorderControl>,
        player: Box<dyn PlayerControl>,
        on_baseline: BaselineSink,
        pictures: Box<dyn PictureSource>,
        on_lost: feed::OnLost,
        patches: [std::sync::Arc<compose::PatchBox>; 3],
    ) -> Self {
        Self { pump, recorder, player, plane: None, snr_mode: None, ae_chn: None, fps_now: 0, ae_limit_logged: false, encoder: None, hdmi_open: true, baseline: None, on_baseline, pictures, feeder: feed::Feeder::default(), lost: feed::Lost::default(), on_lost, patches, rings: Vec::new() }
    }

    fn fail(op: &Op, detail: String) -> HalError {
        HalError { op: format!("{op:?}"), detail }
    }

    fn mi(op: &Op, r: crate::Result<()>) -> Result<(), HalError> {
        r.map_err(|e: MiError| HalError { op: format!("{op:?}"), detail: e.to_string() })
    }

    fn io(op: &Op, r: std::io::Result<()>) -> Result<(), HalError> {
        r.map_err(|e| HalError { op: format!("{op:?}"), detail: e.to_string() })
    }

    /// The compositor of an output's canvases for the frames of `src` at `fps`, with the `ui`'s still sets of its output
    /// (the box of its pool).
    fn composer(&self, src: ChnPort, dst: ChnPort, fps: u32, way: &FeedWay) -> Result<compose::Composer, String> {
        let FeedWay::Canvas { pool, size, live, view, visible, windows } = way;
        let t0 = std::time::Instant::now();
        let patches = self.patches.get(*pool as usize).ok_or_else(|| format!("no patches for pool {pool}"))?.clone();
        let c = compose::Composer::start(*pool, src, dst, *size, fps, *live, *view, visible, windows, patches).map_err(|e| e.to_string())?;
        log::info!("compositor started with {} picture windows in {} ms", windows.len(), t0.elapsed().as_millis());
        Ok(c)
    }

    /// An encoder input fed, read or let go anew: whatever was found lost of it before is past.
    fn fresh(&self, to: ChnPort) {
        if to.module == microcam_core::reconcile::Module::Venc {
            self.lost.clear(to.chn);
        }
    }

    fn plane(&mut self, op: &Op) -> Result<sensor::PlaneInfo, HalError> {
        if let Some(p) = self.plane {
            return Ok(p);
        }
        let p = sensor::plane_info(topo::PAD).map_err(|e| HalError { op: format!("{op:?}"), detail: e.to_string() })?;
        log::info!("sensor plane: cap {}x{}+{}+{} pixel {}", p.cap.w, p.cap.h, p.cap.x, p.cap.y, p.pixel);
        self.plane = Some(p);
        Ok(p)
    }
}

impl Hal for MiHal {
    fn frames(&mut self) -> Option<u64> {
        vif::interrupts(topo::VIF_DEV, topo::VIF_PORT).ok().map(u64::from)
    }

    fn encoder_lost(&mut self, chn: u32) -> bool {
        self.lost.is(chn)
    }

    fn after_release(&mut self, pool: u32, reader: ChnPort) -> Option<(u64, u64, u64)> {
        crate::canvas::after_release(pool, reader).map_err(|e| log::warn!("canvas phase read: {e}")).ok()
    }

    fn early(&mut self, disp: ChnPort, on: bool) {
        self.feeder.early(disp, on);
    }

    fn frame_lines(&mut self) -> Option<u32> {
        if !sysfs::has_fps_live() {
            return None; // a vendor driver: no frame length to read, no display lock
        }
        sysfs::frame_lines().map_err(|e| log::warn!("frame length read: {e}")).ok()
    }

    fn set_frame_lines(&mut self, lines: u32) -> Result<(), HalError> {
        if !sysfs::has_fps_live() {
            return Ok(());
        }
        sysfs::set_frame_lines(lines).map_err(|e| HalError { op: format!("frame length {lines}"), detail: e.to_string() })
    }

    fn exposure_now(&mut self) -> Option<microcam_core::tuning::ExposureNow> {
        tune::exposure_now(topo::ISP_DEV, topo::ISP_CHN).map_err(|e| log::warn!("exposure read: {e}")).ok()
    }

    fn white_balance_now(&mut self) -> Option<microcam_core::tuning::WhiteBalanceNow> {
        tune::white_balance_now(topo::ISP_DEV, topo::ISP_CHN).map_err(|e| log::warn!("white balance read: {e}")).ok()
    }

    fn shoot(&mut self, src: ChnPort, dev: u32, chn: u32, codec: microcam_core::Codec, fps: u32, way: &FeedWay, from: Option<u64>, seq: u64, timeout_ms: u32) -> Result<Vec<u8>, HalError> {
        let err = |detail: String| HalError { op: format!("shoot {dev}/{chn}"), detail };
        let mut c = self.composer(src, topo::venc_in(dev, chn), fps, way).map_err(err)?;
        let shot = c.shoot(src, from, seq, timeout_ms);
        let stopped = c.stop();
        shot.and(stopped).map_err(err)?;
        let s = venc::get_stream(dev, chn, codec, timeout_ms as i32).map_err(|e| err(e.to_string()))?.ok_or_else(|| err(format!("no frame in {timeout_ms} ms")))?;
        let mut buf = vec![0u8; s.len()];
        let n = s.copy_to(&mut buf);
        venc::release(dev, chn, s).map_err(|e| err(e.to_string()))?;
        buf.truncate(n);
        Ok(buf)
    }

    fn snapshot(&mut self, src: ChnPort, feed: ChnPort, snap: ChnPort, from: Option<u64>, seq: u64, timeout_ms: u32) -> Result<Vec<u8>, HalError> {
        let err = |detail: String| HalError { op: format!("snapshot {}/{}", snap.dev, snap.chn), detail };
        let (tx, rx) = std::sync::mpsc::channel();
        let wait = std::time::Duration::from_millis(timeout_ms as u64);
        let s = compose::Snap { venc: snap, port: src, from, seq, deadline: std::time::Instant::now() + wait, reply: tx };
        self.feeder.snapshot(feed, s).map_err(err)?;
        rx.recv_timeout(wait + std::time::Duration::from_millis(500)).map_err(|_| err("the feed did not answer".into()))?.map_err(err)?;
        let s = venc::get_stream(snap.dev, snap.chn, microcam_core::Codec::Mjpeg, timeout_ms as i32).map_err(|e| err(e.to_string()))?.ok_or_else(|| err(format!("no JPEG in {timeout_ms} ms")))?;
        let mut buf = vec![0u8; s.len()];
        let n = s.copy_to(&mut buf);
        venc::release(snap.dev, snap.chn, s).map_err(|e| err(e.to_string()))?;
        buf.truncate(n);
        Ok(buf)
    }

    fn grab(&mut self, src: ChnPort, size: Size, into: microcam_core::desired::PictureId, stretch_up: bool, timeout_ms: u32) -> Result<(), HalError> {
        let err = |detail: String| HalError { op: format!("grab {}", into.0), detail };
        if !stretch_up {
            scl::create_device(topo::SCL_STRETCH_DEV, topo::SCL_STRETCH_HW_MASK).map_err(|e| err(e.to_string()))?;
        }
        let frame = crate::shot::grab(src, size, timeout_ms as i32);
        if !stretch_up {
            if let Err(e) = scl::destroy_device(topo::SCL_STRETCH_DEV) {
                log::warn!("grab: the scaler's device: {e}");
            }
        }
        self.pictures.grabbed(into, frame.map_err(err)?);
        Ok(())
    }

    fn exec(&mut self, op: &Op) -> Result<(), HalError> {
        log::debug!("exec {op:?}");
        use Op::*;
        match op {
            SensorSetRes { pad, index, .. } => {
                self.plane = None;
                Self::mi(op, sensor::set_plane_mode(*pad, false))?;
                self.snr_mode = sensor::modes(*pad).ok().and_then(|l| l.into_iter().find(|d| d.index == *index)).map(|d| (*pad, d.max_fps));
                self.fps_now = self.snr_mode.map_or(0, |(_, f)| f);
                Self::mi(op, sensor::set_res(*pad, *index))
            }
            SensorEnable { pad } => {
                let r = Self::mi(op, sensor::enable(*pad));
                self.ae_limit();
                r
            }
            SensorDisable { pad } => {
                self.plane = None;
                Self::mi(op, sensor::disable(*pad))
            }
            SensorFpsLive { fps } if sysfs::has_fps_live() => {
                self.fps_now = if *fps == 0 { self.snr_mode.map_or(0, |(_, f)| f) } else { *fps };
                self.ae_limit();
                Self::io(op, sysfs::set_fps_live(*fps))
            }
            SensorFpsLive { fps } => {
                let (pad, max) = self.snr_mode.ok_or_else(|| HalError { op: format!("{op:?}"), detail: "no sensor mode set".into() })?;
                Self::mi(op, sensor::set_fps(pad, if *fps == 0 { max } else { (*fps).min(max) }))
            }
            SensorOrientation { pad, mirror, flip } => Self::io(op, sysfs::set_orientation(*pad, *mirror, *flip)),

            VifCreateGroup { group } => Self::mi(op, vif::create_group(*group)),
            VifDestroyGroup { group } => Self::mi(op, vif::destroy_group(*group)),
            VifConfigureDev { dev } => {
                let p = self.plane(op)?;
                Self::mi(op, vif::set_dev_attr(*dev, &p))
            }
            VifEnableDev { dev } => Self::mi(op, vif::enable_dev(*dev)),
            VifDisableDev { dev } => Self::mi(op, vif::disable_dev(*dev)),
            VifConfigurePort { dev, port } => {
                let p = self.plane(op)?;
                Self::mi(op, vif::set_port_attr(*dev, *port, &p))
            }
            VifEnablePort { dev, port } => Self::mi(op, vif::enable_port(*dev, *port)),
            VifDisablePort { dev, port } => Self::mi(op, vif::disable_port(*dev, *port)),

            IspCreateDevice { dev } => Self::mi(op, isp::create_device(*dev)),
            IspDestroyDevice { dev } => {
                self.baseline = None;
                Self::mi(op, isp::destroy_device(*dev))
            }
            IspCreateChannel { dev, chn, pad } => Self::mi(op, isp::create_channel(*dev, *chn, *pad)),
            IspDestroyChannel { dev, chn } => {
                self.baseline = None;
                Self::mi(op, isp::destroy_channel(*dev, *chn))
            }
            IspSetChnParam { dev, chn, nr3d } => Self::mi(op, isp::set_chn_param(*dev, *chn, *nr3d)),
            IspStartChannel { dev, chn } => Self::mi(op, isp::start_channel(*dev, *chn)),
            IspStopChannel { dev, chn } => Self::mi(op, isp::stop_channel(*dev, *chn)),
            IspConfigurePort { dev, chn, port } => Self::mi(op, isp::set_port_param(*dev, *chn, *port, None)),
            IspEnablePort { dev, chn, port } => Self::mi(op, isp::enable_port(*dev, *chn, *port)),
            IspDisablePort { dev, chn, port } => Self::mi(op, isp::disable_port(*dev, *chn, *port)),
            IspLoadIq { dev, chn, path } => {
                // as the vendor app: wait until AE and AWB report ready (≤ 101 × 10 ms), then load. A failed load is
                // logged loudly but does not fail the transition: the picture then runs on the built-in IQ table.
                match isp::wait_3a_ready(*dev, *chn, 101) {
                    Some(n) => log::info!("3A ready after {} ms", n * 10),
                    None => log::warn!("3A not ready after 1 s; loading the IQ bin anyway"),
                }
                match isp::load_iq_bin(*dev, *chn, path) {
                    Ok(()) => log::info!("IQ bin loaded: {path}"),
                    Err(e) => log::error!("IQ bin NOT loaded ({e}); image tuning is the built-in default"),
                }
                self.baseline = match tune::Baseline::read(*dev, *chn) {
                    Ok(b) => {
                        log::info!("{}", b.describe());
                        (self.on_baseline)(b.hardware_defaults());
                        self.ae_chn = Some((*dev, *chn));
                        Some(b)
                    }
                    Err(e) => {
                        log::error!("picture knobs unavailable: reading the IQ baseline failed ({e})");
                        None
                    }
                };
                self.ae_limit();
                Ok(())
            }
            IspAdjust { dev, chn, adjust } => {
                let r = match &self.baseline {
                    Some(b) => tune::apply(*dev, *chn, b, adjust).map_err(|e| e.to_string()),
                    None => Err("no IQ baseline".into()),
                };
                match &r {
                    Ok(()) => log::info!("picture: {adjust:?}"),
                    Err(e) => log::error!("picture: {adjust:?} refused ({e})"),
                }
                r.map_err(|e| Self::fail(op, e))
            }

            SclCreateDevice { dev, hw_mask } => Self::mi(op, scl::create_device(*dev, *hw_mask)),
            SclDestroyDevice { dev } => Self::mi(op, scl::destroy_device(*dev)),
            SclCreateChannel { dev, chn } => Self::mi(op, scl::create_channel(*dev, *chn)),
            SclDestroyChannel { dev, chn } => Self::mi(op, scl::destroy_channel(*dev, *chn)),
            SclStartChannel { dev, chn } => Self::mi(op, scl::start_channel(*dev, *chn)),
            SclStopChannel { dev, chn } => Self::mi(op, scl::stop_channel(*dev, *chn)),
            SclConfigurePort { dev, chn, port, crop, size } => Self::mi(op, scl::set_port_param(*dev, *chn, *port, *crop, *size)),
            SclEnablePort { dev, chn, port } => Self::mi(op, scl::enable_port(*dev, *chn, *port)),
            SclDisablePort { dev, chn, port } => Self::mi(op, scl::disable_port(*dev, *chn, *port)),
            SclClock { hz } => Self::io(op, sysfs::set_scl_clock(*hz)),

            Bind { src, dst, kind, src_fps, dst_fps } => Self::mi(op, sys::bind(*src, *dst, *kind, *src_fps, *dst_fps)),
            Unbind { src, dst } => Self::mi(op, sys::unbind(*src, *dst)),
            SetDepth { port, user, buffers } => Self::mi(op, sys::set_depth(*port, *user, *buffers)),
            PoolUp { id, src, readers, size, rows, n } => {
                crate::canvas::up(*id, *size, *rows, *n, *src, readers).map_err(|e| Self::fail(op, e))?;
                // the whole canvas to the port until a feed lends it otherwise (a photo's port has no feed)
                crate::canvas::view(*id, Some((0, 0))).map_err(|e| Self::fail(op, e))
            }
            PoolCount { id, n } => crate::canvas::count(*id, *n).map(|_| ()).map_err(|e| Self::fail(op, e)),
            PoolDown { id } => crate::canvas::down(*id).map_err(|e| Self::fail(op, e)),
            PoolReader { id, reader, fps } => {
                self.fresh(*reader);
                // the photo's encoder beside a recording takes the one canvas its compositor names for it
                let once = reader.module == microcam_core::reconcile::Module::Venc && reader.chn == topo::VENC_CHN_SNAP;
                let reads = if once { crate::canvas::Reads::Once } else { crate::canvas::Reads::At(*fps) };
                crate::canvas::reader(*id, *reader, reads).map_err(|e| Self::fail(op, e))
            }
            PoolUnreader { id, reader } => {
                self.fresh(*reader);
                crate::canvas::unreader(*id, *reader).map_err(|e| Self::fail(op, e))
            }

            HdmiDispUp { timing } => {
                // the vendor app's order (its init_hdmi): HDMI first, then DISP; input port 0 the whole screen, its frames
                // the HDMI pool's canvases at the screen's size
                if !self.hdmi_open {
                    Self::mi(op, hdmi::open())?;
                    self.hdmi_open = true;
                }
                Self::mi(op, hdmi::set_timing(*timing))?;
                Self::mi(op, hdmi::start())?;
                let screen = timing.size();
                Self::mi(op, disp::up(topo::DISP_DEV, topo::DISP_LAYER, topo::DISP_PORT, *timing, Rect::full(screen), screen))
            }
            FeedStart { src, dst, fps, way } => {
                let way = self.composer(*src, *dst, *fps, way).map_err(|e| Self::fail(op, e))?;
                self.fresh(*dst);
                Self::mi(op, self.feeder.start(*src, *dst, way, self.lost.clone(), self.on_lost.clone()))
            }
            FeedStop { dst } => {
                self.fresh(*dst);
                self.feeder.stop(*dst).map_err(|e| Self::fail(op, e))
            }
            HdmiDispDown => {
                let r1 = disp::down(topo::DISP_DEV, topo::DISP_LAYER, topo::DISP_PORT);
                let r2 = hdmi::stop();
                Self::mi(op, r1.and(r2))
            }

            VencUp { dev, chn, codec, size, fps, mjpeg_quality, bitrate, gop, bound } => {
                let spec = venc::EncoderSpec { codec: *codec, size: *size, fps: *fps, mjpeg_quality: *mjpeg_quality, bitrate: *bitrate, gop: *gop, ring: microcam_core::heap::jpeg_ring(*chn, *size, *fps) as u32, start_floor: *chn == topo::VENC_CHN_MEDIA };
                Self::mi(op, venc::up(*dev, *chn, &spec, *bound))?;
                self.rings.retain(|(c, _)| c != chn);
                if let Some(r) = bound {
                    self.rings.push((*chn, r.port));
                }
                if *chn == topo::VENC_CHN_USB {
                    self.encoder = Some((*dev, *chn, *codec));
                }
                Ok(())
            }
            VencDown { dev, chn } => {
                if *chn == topo::VENC_CHN_USB {
                    self.encoder = None;
                }
                let ring = self.rings.iter().find(|(c, _)| c == chn).map(|(_, p)| *p);
                self.rings.retain(|(c, _)| c != chn);
                Self::mi(op, venc::down(*dev, *chn, ring))
            }

            PumpStart { epoch, venc_dev, venc_chn } => {
                let codec = match self.encoder {
                    Some((d, c, codec)) if d == *venc_dev && c == *venc_chn => codec,
                    _ => return Err(HalError { op: format!("{op:?}"), detail: "pump start without a matching encoder".into() }),
                };
                self.pump.start(*epoch, *venc_dev, *venc_chn, codec).map_err(|d| HalError { op: format!("{op:?}"), detail: d })
            }
            PumpStop => self.pump.stop().map_err(|d| HalError { op: format!("{op:?}"), detail: d }),
            RecordStart { venc_dev, venc_chn, codec, path, size, fps } => self.recorder.start(*venc_dev, *venc_chn, *codec, path, *size, *fps).map_err(|d| HalError { op: format!("{op:?}"), detail: d }),
            RecordStop => self.recorder.stop().map_err(|d| HalError { op: format!("{op:?}"), detail: d }),
            VdecUp { codec, size, ring, port } => Self::mi(op, crate::vdec::up(*codec, *size, *ring, *port)),
            VdecPort { port } => Self::mi(op, crate::vdec::set_port(*port)),
            VdecDown => Self::mi(op, crate::vdec::down()),
            PlayStart { path, .. } => self.player.start(path).map_err(|d| Self::fail(op, d)),
            PlayStop => self.player.stop().map_err(|d| Self::fail(op, d)),
        }
    }
}
