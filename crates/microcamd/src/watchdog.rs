//! The hardware watchdog, fed from the daemon's own view of its health. The pipeline thread (the one owner of the MI
//! graph) marks itself healthy on every tick in which it is not stuck and, while a capture is built, the sensor's frames
//! keep arriving; a feeder thread feeds the device while that mark is fresh. A hung MI call, a stuck pipeline thread, a
//! sensor that stopped or a crashed daemon all end in a hardware reset; a pipeline that failed to build but is still
//! answering does not (it stays reachable for diagnosis).

use std::io::Write;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// How long without a health mark before feeding stops. Above any legitimate pipeline step (a full rebuild with the
/// IQ load takes a few seconds) and above several pipeline ticks.
pub const STALE: Duration = Duration::from_secs(30);
/// Hardware timeout asked of the driver; the reset follows this long after the last feed.
const HW_TIMEOUT_S: i32 = 30;
const FEED_EVERY: Duration = Duration::from_secs(2);
/// `WDIOC_SETTIMEOUT` = `_IOWR('W', 6, int)`
const WDIOC_SETTIMEOUT: libc::c_ulong = 0xC004_5706;

pub struct Health {
    start: Instant,
    /// ms since `start` of the last health mark
    last_ok_ms: AtomicU64,
}

impl Health {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { start: Instant::now(), last_ok_ms: AtomicU64::new(0) })
    }

    pub fn ok(&self) {
        self.last_ok_ms.store(self.start.elapsed().as_millis() as u64, Ordering::Relaxed);
    }

    fn age(&self) -> Duration {
        self.start.elapsed().saturating_sub(Duration::from_millis(self.last_ok_ms.load(Ordering::Relaxed)))
    }
}

/// The open device; dropping it without [`Watchdog::disarm`] leaves the timer running (a crash resets the camera).
pub struct Watchdog {
    file: Arc<std::fs::File>,
    stop: Arc<AtomicBool>,
    feeder: thread::JoinHandle<()>,
}

/// Open the device and start feeding. None when there is no device configured or another feeder holds it (a trial
/// run under the vendor boot chain); the daemon then runs unwatched and says so.
pub fn start(device: &Path, health: Arc<Health>) -> Option<Watchdog> {
    if device.as_os_str().is_empty() {
        log::info!("watchdog: off (no device configured)");
        return None;
    }
    let file = match std::fs::OpenOptions::new().write(true).open(device) {
        Ok(f) => Arc::new(f),
        Err(e) => {
            log::warn!("watchdog {}: not opened ({e}); another feeder may hold it — running unwatched", device.display());
            return None;
        }
    };
    let mut t = HW_TIMEOUT_S;
    // SAFETY: WDIOC_SETTIMEOUT reads and writes one int.
    if unsafe { libc::ioctl(file.as_raw_fd(), WDIOC_SETTIMEOUT as _, &mut t) } < 0 {
        log::warn!("watchdog: timeout not set ({}); the driver's own applies", std::io::Error::last_os_error());
    } else {
        log::info!("watchdog {}: armed, hardware timeout {t} s, feeding while healthy (stale after {} s)", device.display(), STALE.as_secs());
    }
    let f = file.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let st = stop.clone();
    let feeder = thread::Builder::new()
        .name("watchdog".into())
        .spawn(move || {
            let mut feeding = true;
            while !st.load(Ordering::SeqCst) {
                let fresh = health.age() < STALE;
                if fresh {
                    let _ = (&*f).write_all(b"\0");
                }
                if fresh != feeding {
                    if fresh {
                        log::warn!("watchdog: healthy again, feeding");
                    } else {
                        log::error!("watchdog: no health mark for {} s — not feeding, the hardware will reset the camera", health.age().as_secs());
                        crate::logger::dump("watchdog stopped feeding");
                    }
                    feeding = fresh;
                }
                thread::sleep(FEED_EVERY);
            }
        })
        .ok()?;
    Some(Watchdog { file, stop, feeder })
}

impl Watchdog {
    /// A deliberate stop: the magic close character asks the driver to stop the timer (a driver built with
    /// `nowayout` ignores it, and the camera then resets after the timeout).
    /// The feeder stops first: any write after the 'V' would cancel it.
    pub fn disarm(self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.feeder.join();
        let _ = (&*self.file).write_all(b"V");
        drop(self.file);
        log::info!("watchdog: disarmed");
    }
}
