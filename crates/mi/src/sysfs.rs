//! The non-MI knobs the pipeline needs: the sensor driver's live frame rate and frame length, the sensor orientation through the
//! mi_sensor procfs, the scaler clock.

use std::fs;
use std::io;

pub const FPS_LIVE: &str = "/sys/module/drv_ms_cus_gc8613_MIPI/parameters/fps_live";
pub const VTS_LIVE: &str = "/sys/module/drv_ms_cus_gc8613_MIPI/parameters/vts_live";
pub const VTS_NOW: &str = "/sys/module/drv_ms_cus_gc8613_MIPI/parameters/ae_vts";
pub const SENSOR_PROC: &str = "/proc/mi_modules/mi_sensor/mi_sensor0";
pub const SCL_CLOCK: &str = "/sys/devices/virtual/mstar/mscl/clk";

/// Frame length from the next frame on: the driver computes lines_per_second / fps, never shorter than the mode's own
/// frame. 0 restores the mode's rate: written as a rate above every mode's, which the driver clamps to the mode's own
/// frame — its own 0 means «nothing pending» and would leave the last length (a 4K45 stream after 30-Hz idle ran at
/// 30, 2026-11-11). Only our driver has this parameter.
pub fn set_fps_live(fps: u32) -> io::Result<()> {
    let fps = if fps == 0 { 1000 } else { fps };
    fs::write(FPS_LIVE, format!("{fps}\n"))
}

/// Frame length in lines from the next frame on (same mode and line time). Only our driver has this parameter.
pub fn set_frame_lines(lines: u32) -> io::Result<()> {
    fs::write(VTS_LIVE, format!("{lines}\n"))
}

/// The frame length in lines in effect.
pub fn frame_lines() -> io::Result<u32> {
    let s = fs::read_to_string(VTS_NOW)?;
    s.trim().parse().map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "ae_vts not a number"))
}

/// `setmirrorflip <pad> <mirror> <flip>` — the orientation the vendor app sets once after start (`0 1 1`); a
/// re-created sensor starts flipped unless it is repeated.
pub fn set_orientation(pad: u32, mirror: bool, flip: bool) -> io::Result<()> {
    fs::write(SENSOR_PROC, format!("setmirrorflip {pad} {} {}\n", mirror as u8, flip as u8))
}

pub fn set_scl_clock(hz: u32) -> io::Result<()> {
    fs::write(SCL_CLOCK, format!("{hz}\n"))
}

pub fn scl_clock() -> io::Result<u32> {
    let s = fs::read_to_string(SCL_CLOCK)?;
    s.trim().parse().map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "scl clock not a number"))
}
