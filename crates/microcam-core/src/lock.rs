//! The sensor locked to the display.
//!
//! DISP takes a new frame only on its pass after a vsync and shows it from the vsync after that, so a canvas handed
//! just before the pass waits one display period and one handed just after it waits two. While HDMI shows the live
//! picture of a sensor running at the display's rate, the sensor's frame length is set each tick so that the
//! hand-over moves towards [`LEAD_US`] before DISP's pass. Both come from our canvas module, in the camera's own clock:
//! the phase — each hand-over's time after DISP's last release, which DISP makes on that pass — and the display's
//! period, from the releases' times (the nominal period is off by the two clocks' difference, enough to make a whole
//! line count match the display and stop the phase from moving at all).
//!
//! Locked, DISP may also be handed the canvas the port is about to write ([`early`]): the port is lent a
//! canvas ~34 ms before it writes the canvas's first row, so DISP, handed it then, scans it from the vsync after its
//! pass, a few ms after the write's start, and follows the write.

use crate::hdmi::Timing;
use crate::modes::Mode;
use crate::reconcile::{topo, ChnPort, Current, FeedWay};

/// How long before DISP's pass the hand-over is aimed: the hand-overs scatter by up to about 2 ms around their mean
/// (measured 2026-11-11), and one that misses the pass waits a whole period more.
pub const LEAD_US: f64 = 5_000.0;

/// How far off the aim the phase may rest for DISP to be handed a canvas before the port writes it: the lead less this
/// keeps DISP's pass a few ms after the lending, so its scan starts after the write's.
pub const EARLY_WITHIN_US: f64 = 2_000.0;

/// How much longer the write may take than DISP's scan of the frame: its start ahead of the scan (the lead less
/// [`EARLY_WITHIN_US`]) covers it.
pub const EARLY_SLACK_NS: u64 = 500_000;

/// What the lock steers: DISP's reader of HDMI's pool, at the display's nominal period, with the sensor's line time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lock {
    pub pool: u32,
    pub disp: ChnPort,
    pub period_ns: u64,
    pub line_ns: u32,
    /// the mode's own frame: no shorter one exists
    pub min_lines: u32,
    /// how long the port takes to write a frame — the sensor's readout of its rows; None when the port does not
    /// write as the sensor reads out (not the realtime SCL dev 0: a zoom goes through memory)
    pub write_ns: Option<u64>,
    /// how long DISP takes to scan a frame's rows; None when the timing's line count is not known
    pub scan_ns: Option<u64>,
}

/// The lock while HDMI shows the live picture (its feed's port writes the canvases) of a sensor running at the
/// display's rate; None otherwise — a covered or paused HDMI frame is handed on its own clock, and a sensor at another
/// rate is thinned to the display's.
pub fn lock(cur: &Current, mode: &Mode) -> Option<Lock> {
    let timing: Timing = cur.hdmi.as_ref()?.timing;
    let sensor = cur.sensor.as_ref().filter(|s| s.enabled)?;
    let fps = if sensor.fps_live == 0 { mode.max_fps } else { sensor.fps_live.min(mode.max_fps) };
    if fps != timing.rate_hz() {
        return None;
    }
    let feed = cur.feeds.iter().find(|f| f.dst == topo::DISP_IN)?;
    let FeedWay::Canvas { pool, view: Some(_), .. } = feed.way else { return None };
    let period_ns = 1_000_000_000 / timing.rate_hz() as u64;
    let realtime = feed.src.module == crate::reconcile::Module::Scl && feed.src.dev == 0;
    Some(Lock {
        pool,
        disp: topo::DISP_IN,
        period_ns,
        line_ns: mode.line_ns,
        min_lines: mode.frame_lines,
        write_ns: realtime.then(|| mode.height as u64 * mode.line_ns as u64),
        scan_ns: timing.v_total().map(|t| period_ns * timing.size().h as u64 / t as u64),
    })
}

/// The display's period in the camera's clock, from two of DISP's releases `span_ns` apart: the whole number of nominal
/// periods between them (the clocks differ by far less than half a period over a tick).
pub fn period(l: &Lock, span_ns: u64) -> Option<f64> {
    let n = (span_ns as f64 / l.period_ns as f64).round();
    (n >= 1.0).then(|| span_ns as f64 / n)
}

/// The frame length for the next tick of `frames` frames: the display's period in lines plus half the phase error
/// spread over the tick (half: the mean phase of a tick lags its end), at most a line and a half off the period — a
/// frame length one line off the display's moves the phase by a line a frame at most.
pub fn frame_lines(l: &Lock, period_ns: f64, mean_after_us: f64, frames: f64) -> u32 {
    let lines = period_ns / l.line_ns as f64;
    let error_us = error_us(l, mean_after_us);
    let step = (0.5 * error_us * 1000.0 / (frames.max(1.0) * l.line_ns as f64)).clamp(-1.5, 1.5);
    ((lines + step).round() as u32).max(l.min_lines)
}

/// The hand-overs' mean phase off the aim (µs; positive: early).
fn error_us(l: &Lock, mean_after_us: f64) -> f64 {
    l.period_ns as f64 / 1000.0 - LEAD_US - mean_after_us
}

/// DISP may be handed the canvas the port is about to write: the port writes as the sensor reads out, no slower than
/// DISP scans, and the hand-overs rest near the aim.
pub fn early(l: &Lock, mean_after_us: f64) -> bool {
    let (Some(write), Some(scan)) = (l.write_ns, l.scan_ns) else { return false };
    write <= scan + EARLY_SLACK_NS && error_us(l, mean_after_us).abs() <= EARLY_WITHIN_US
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modes::ModeTable;
    use crate::reconcile::{FeedState, HdmiState, SensorState};
    use crate::{Rect, Size};

    fn mode() -> Mode {
        let m = ModeTable::parse(crate::modes::tests::modes_toml()).unwrap().modes;
        m.into_iter().find(|m| m.name == "3840x2160@40 rk fastpll").unwrap()
    }

    fn live(view: Option<Rect>, fps_live: u32) -> Current {
        let mut c = Current { hdmi: Some(HdmiState { timing: Timing::P2160_30 }), ..Default::default() };
        c.sensor = Some(SensorState { mode: "3840x2160@40 rk fastpll".into(), enabled: true, fps_live, ..Default::default() });
        let r = Rect { x: 0, y: 0, w: 3840, h: 2160 };
        c.feeds.push(FeedState {
            src: ChnPort::new(crate::reconcile::Module::Scl, 0, 0, 0),
            dst: topo::DISP_IN,
            fps: 30,
            way: FeedWay::Canvas { pool: 2, size: Size::new(3840, 2160), live: r, view, visible: vec![r], windows: vec![] },
        });
        c
    }

    #[test]
    fn locks_a_live_hdmi_at_the_sensor_rate() {
        let l = lock(&live(Some(Rect { x: 0, y: 0, w: 3840, h: 2160 }), 30), &mode()).unwrap();
        assert_eq!((l.pool, l.period_ns), (2, 33_333_333));
        // the display at 33 332.0 us in the camera's clock (measured): 3085.4 lines
        let p = period(&l, 5_000_000_000 - 120_000).unwrap();
        assert!((p - 33_332_533.0).abs() < 1.0, "{p}");
        // on the aim: the nearest whole count; far early: a line and a half more; far late: less
        assert_eq!(frame_lines(&l, 33_332_000.0, 28_333.0, 150.0), 3085);
        assert_eq!(frame_lines(&l, 33_332_000.0, 5_000.0, 150.0), 3087);
        assert_eq!(frame_lines(&l, 33_332_000.0, 33_000.0, 150.0), 3084);
    }

    #[test]
    fn early_only_near_the_aim_and_with_the_write_ahead_of_the_scan() {
        let mut l = lock(&live(Some(Rect { x: 0, y: 0, w: 3840, h: 2160 }), 30), &mode()).unwrap();
        // fastpll: 2160 rows in ~23 ms, the 2160p30 scan 32 ms
        assert!(early(&l, 28_333.0));
        assert!(early(&l, 26_500.0));
        assert!(!early(&l, 25_000.0));
        assert!(!early(&l, 31_000.0));
        // a write slower than the scan
        l.write_ns = Some(33_000_000);
        assert!(!early(&l, 28_333.0));
        l.write_ns = None;
        assert!(!early(&l, 28_333.0));
    }

    #[test]
    fn no_lock_when_covered_or_at_another_rate() {
        assert_eq!(lock(&live(None, 30), &mode()), None);
        assert_eq!(lock(&live(Some(Rect { x: 0, y: 0, w: 3840, h: 2160 }), 25), &mode()), None);
        // fps_live 0: the mode's own 40
        assert_eq!(lock(&live(Some(Rect { x: 0, y: 0, w: 3840, h: 2160 }), 0), &mode()), None);
    }
}
