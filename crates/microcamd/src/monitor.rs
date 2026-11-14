//! HDMI monitor: hot-plug events from the transmitter's callback, EDID → `Monitor`.

use std::sync::mpsc::{Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use microcam_core::hdmi::Monitor;

use crate::pipeline::Msg;

/// Read connection state and EDID now.
pub fn probe() -> Monitor {
    match mi::hdmi::connected() {
        Ok(false) => return Monitor::Absent,
        Ok(true) => {}
        Err(e) => {
            log::warn!("HDMI connect state unreadable ({e}); assuming absent");
            return Monitor::Absent;
        }
    }
    match mi::hdmi::edid() {
        Ok(Some(bytes)) => match microcam_core::edid::parse(&bytes) {
            Some(caps) => Monitor::Present(caps),
            None => {
                log::warn!("EDID {} bytes did not parse", bytes.len());
                Monitor::PresentNoEdid
            }
        },
        Ok(None) => Monitor::PresentNoEdid,
        Err(e) => {
            log::warn!("EDID read failed ({e})");
            Monitor::PresentNoEdid
        }
    }
}

/// Register the hot-plug callback. Each event is debounced on a helper thread (a monitor input switch looks like
/// unplug + replug within a second) and the resulting `Monitor` goes to the pipeline.
pub fn start(tx: Sender<Msg>) -> Result<(), mi::MiError> {
    let (etx, erx) = std::sync::mpsc::channel::<mi::hdmi::Event>();
    mi::hdmi::init(Box::new(move |ev| {
        let _ = etx.send(ev);
    }))?;
    thread::Builder::new()
        .name("hdmi".into())
        .spawn(move || {
            let mut next = erx.recv().ok();
            while let Some(ev) = next.take() {
                let mut last = ev;
                // coalesce a burst
                while let Ok(e) = erx.recv_timeout(Duration::from_millis(600)) {
                    last = e;
                }
                log::info!("HDMI event: {last:?}");
                let m = match last {
                    mi::hdmi::Event::Unplugged => Monitor::Absent,
                    mi::hdmi::Event::Plugged => {
                        // the EDID is readable a moment after the plug
                        thread::sleep(Duration::from_millis(300));
                        probe()
                    }
                };
                let no_edid = m == Monitor::PresentNoEdid;
                if tx.send(Msg::Monitor(m)).is_err() {
                    return;
                }
                next = if no_edid { edid_late(&erx, &tx) } else { None };
                if next.is_none() {
                    next = erx.recv().ok();
                }
            }
        })
        .expect("spawn hdmi thread");
    Ok(())
}

/// After a cable plug following a long absence the EDID read fails until HDMI runs again (an input switch, where the
/// transmitter was stopped for under a second, reads it at once). The pipeline already shows the no-EDID timing; keep
/// re-reading for a while and hand over the monitor once its EDID answers. A new event ends the wait and is returned.
fn edid_late(erx: &Receiver<mi::hdmi::Event>, tx: &Sender<Msg>) -> Option<mi::hdmi::Event> {
    const RETRY: Duration = Duration::from_secs(1);
    const WAIT: Duration = Duration::from_secs(30);
    let t0 = Instant::now();
    while t0.elapsed() < WAIT {
        if let Ok(ev) = erx.recv_timeout(RETRY) {
            return Some(ev);
        }
        match probe() {
            Monitor::PresentNoEdid => {}
            Monitor::Absent => return None,
            m => {
                log::info!("HDMI: EDID read {} ms after the fallback", t0.elapsed().as_millis());
                let _ = tx.send(Msg::Monitor(m));
                return None;
            }
        }
    }
    log::warn!("HDMI: no EDID within {} s, staying on the no-EDID timing", WAIT.as_secs());
    None
}
