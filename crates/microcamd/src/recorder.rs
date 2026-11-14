//! The recorder: the media encoder's H.264 or H.265 → a fragmented MP4 on the SD card. Two threads per recording: the fetcher
//! only takes frames from the encoder, so `stop` (the barrier before the encoder is touched) is quick; the writer muxes
//! and writes, so a slow card holds up only itself. A fragment is flushed to the card at every IDR. The video's cover,
//! put into [`Covers`] by the pipeline once taken, is written by the writer between two frames over the reserve the init
//! segment holds for it (`mp4::cover_box`). When the writer ends, `closed` hears the file it closed whole, or why it
//! did not.

use std::fs::File;
use std::io::Write;
use std::os::unix::fs::FileExt;
use std::sync::Mutex;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use microcam_core::mp4::{self, Muxer};
use microcam_core::{Codec, Size};
use mi::hal::RecorderControl;
use mi::venc;

/// Frames the writer may fall behind the encoder (a few seconds) before the fetcher drops to the next IDR.
const QUEUE: usize = 180;
const FETCH_WAIT_MS: i32 = 100;

/// A frame for the writer and its pts, or the encoder's error that ends the recording.
type Frame = Result<(Vec<u8>, u64), String>;

/// Told once per recording, when its writer ends: the file closed whole on the card, or why not.
pub type Closed = Arc<dyn Fn(Result<String, String>) + Send + Sync>;

pub struct Recorder {
    running: Option<Running>,
    closed: Closed,
    covers: Arc<Covers>,
}

impl Recorder {
    pub fn new(closed: Closed, covers: Arc<Covers>) -> Self {
        Self { running: None, closed, covers }
    }
}

/// The cover waiting for its recording's writer: the recording's file as asked, and the thumbnail JPEG.
#[derive(Default)]
pub struct Covers(Mutex<Option<(String, Vec<u8>)>>);

impl Covers {
    /// The cover of the recording into `file` (one not taken yet, of an earlier recording, goes).
    pub fn put(&self, file: &str, jpeg: Vec<u8>) {
        *self.0.lock().unwrap() = Some((file.to_string(), jpeg));
    }

    /// The cover of the recording into `file`, if it is here.
    fn take(&self, file: &str) -> Option<Vec<u8>> {
        let mut c = self.0.lock().unwrap();
        c.as_ref().is_some_and(|(f, _)| f == file).then(|| c.take().expect("checked").1)
    }

    fn clear(&self) {
        *self.0.lock().unwrap() = None;
    }
}

struct Running {
    stop: Arc<AtomicBool>,
    /// the fetcher says here that it has left the encoder
    left: Receiver<()>,
}

impl RecorderControl for Recorder {
    fn start(&mut self, dev: u32, chn: u32, codec: Codec, path: &str, size: Size, fps: u32) -> Result<(), String> {
        if self.running.is_some() {
            return Err("already recording".into());
        }
        let (tx, rx) = mpsc::sync_channel(QUEUE);
        let stop = Arc::new(AtomicBool::new(false));
        let (left_tx, left) = mpsc::channel();
        let st = stop.clone();
        thread::Builder::new()
            .name("rec-fetch".into())
            .spawn(move || {
                fetch(dev, chn, codec, &st, tx);
                let _ = left_tx.send(());
            })
            .map_err(|e| e.to_string())?;
        let path = PathBuf::from(path);
        let closed = self.closed.clone();
        self.covers.clear();
        let covers = self.covers.clone();
        thread::Builder::new().name("rec-write".into()).spawn(move || closed(write(&path, codec, size, fps, rx, &covers))).map_err(|e| e.to_string())?;
        log::info!("recording started: {codec:?} {size}@{fps}");
        self.running = Some(Running { stop, left });
        Ok(())
    }

    fn stop(&mut self) -> Result<(), String> {
        let Some(r) = self.running.take() else { return Ok(()) };
        r.stop.store(true, Ordering::Relaxed);
        r.left.recv_timeout(Duration::from_millis(800)).map_err(|_| "recorder did not leave the encoder in 800 ms".to_string())
    }
}

/// Encoder → queue until told to stop. A full queue drops frames up to the next IDR (a P-frame without its reference
/// would smear everything after it). The encoder failing goes to the writer as its failure, which ends the recording.
fn fetch(dev: u32, chn: u32, codec: Codec, stop: &AtomicBool, tx: SyncSender<Frame>) {
    let (mut dropped, mut need_key) = (0u64, false);
    // the rate control held at its start QP until the second IDR: its first GOP (`venc::START_QP`)
    let mut keys = 0u32;
    while !stop.load(Ordering::Relaxed) {
        let s = match venc::get_stream(dev, chn, codec, FETCH_WAIT_MS) {
            Ok(Some(s)) => s,
            Ok(None) => continue,
            Err(e) => {
                log::error!("recorder: {e}");
                let _ = tx.send(Err(format!("the encoder: {e}")));
                break;
            }
        };
        let mut buf = vec![0u8; s.len()];
        let n = s.copy_to(&mut buf);
        buf.truncate(n);
        let (pts, key) = (s.pts(), s.keyframe);
        if key && keys < 2 {
            keys += 1;
            if keys == 2 {
                if let Err(e) = venc::qp_floor(dev, chn, None) {
                    log::warn!("recorder: the rate control's start floor not let go: {e}");
                }
            }
        }
        if let Err(e) = venc::release(dev, chn, s) {
            log::error!("recorder: {e}");
            let _ = tx.send(Err(format!("the encoder: {e}")));
            break;
        }
        if need_key && !key {
            dropped += 1;
            continue;
        }
        need_key = false;
        match tx.try_send(Ok((buf, pts))) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                dropped += 1;
                need_key = true;
            }
            // the writer gave up (card error); keep draining the encoder until the pipeline stops us
            Err(TrySendError::Disconnected(_)) => {}
        }
    }
    if dropped > 0 {
        log::warn!("recorder: {dropped} frames dropped, the card did not keep up");
    }
}

/// Queue → file: init segment at the first IDR that carries the parameter sets, then a fragment per GOP, each
/// flushed to the card.
/// `path`, or `<stem>_2`, `_3`… beside it when taken: a recording restarted after a pipeline failure must not overwrite
/// what it wrote before.
fn free_path(path: &Path) -> PathBuf {
    let (stem, ext) = (path.file_stem().unwrap_or_default().to_string_lossy(), path.extension().map(|e| e.to_string_lossy()));
    (1..)
        .map(|n| match n {
            1 => path.to_path_buf(),
            n => path.with_file_name(format!("{stem}_{n}{}", ext.as_ref().map(|e| format!(".{e}")).unwrap_or_default())),
        })
        .find(|p| !p.exists())
        .unwrap()
}

fn write(path: &Path, codec: Codec, size: Size, fps: u32, rx: Receiver<Frame>, covers: &Covers) -> Result<String, String> {
    let asked = path.to_string_lossy().into_owned();
    let path = free_path(path);
    // where the cover's reserve lies in the file, until the cover is written
    let mut cover_at: Option<u64> = None;
    let mut out: Option<(File, Muxer)> = None;
    let (mut frames, mut bytes, mut first_pts, mut last_pts) = (0u64, 0u64, None, 0u64);
    let result = (|| -> std::io::Result<()> {
        for frame in rx.iter() {
            let (data, pts) = frame.map_err(std::io::Error::other)?;
            let (f, params) = mp4::frame(codec, &data, pts);
            if out.is_none() {
                if !(f.sync && params.complete(codec)) {
                    continue;
                }
                let mux = Muxer::new(size, fps, codec, &params).map_err(std::io::Error::other)?;
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                let mut file = File::create_new(&path)?;
                let init = mux.init();
                file.write_all(&init)?;
                cover_at = Some(mp4::cover_at(init.len()) as u64);
                out = Some((file, mux));
            }
            let (file, mux) = out.as_mut().unwrap();
            if let Some(at) = cover_at {
                if let Some(jpeg) = covers.take(&asked) {
                    cover_at = None;
                    match mp4::cover_box(&jpeg) {
                        // written in place; the next fragment's sync takes it to the card
                        Some(b) => {
                            file.write_all_at(&b, at)?;
                            log::info!("recording {}: cover of {} KB written", path.display(), jpeg.len() >> 10);
                        }
                        None => log::warn!("recording {}: a cover of {} KB does not fit the {} KB reserve; left out", path.display(), jpeg.len() >> 10, mp4::COVER_RESERVE >> 10),
                    }
                }
            }
            first_pts.get_or_insert(pts);
            last_pts = pts;
            frames += 1;
            bytes += f.sample.len() as u64;
            if let Some(frag) = mux.push(f) {
                file.write_all(&frag)?;
                file.sync_data()?;
            }
        }
        if let Some((file, mux)) = out.as_mut() {
            if let Some(frag) = mux.finish() {
                file.write_all(&frag)?;
            }
            file.sync_all()?;
        }
        Ok(())
    })();
    let span_ms = first_pts.map(|f| last_pts.saturating_sub(f) / 1000).unwrap_or(0);
    let r = match result {
        Ok(()) if out.is_some() => {
            log::info!("recording closed: {} — {frames} frames, {span_ms} ms by the encoder clock, {} MB", path.display(), bytes >> 20);
            return Ok(path.to_string_lossy().into_owned());
        }
        Ok(()) => format!("{}: no IDR with parameter sets arrived, nothing written", path.display()),
        Err(e) => format!("{}: {e} after {frames} frames", path.display()),
    };
    log::error!("recording {r}");
    Err(r)
}
