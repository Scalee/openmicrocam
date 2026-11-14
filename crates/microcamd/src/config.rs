//! `config.toml` of the daemon: where its files are, the camera's mounting, its links. Everything has a default that
//! matches the camera as it is today, so an empty file works. The controls' defaults are not here: they are the code's
//! (`Prefs::default`, the knobs' ranges), and what the user sets is the settings file's.

use std::path::{Path, PathBuf};

use microcam_core::policy::Prefs;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Config {
    pub paths: Paths,
    pub mounting: Mounting,
    pub control: Control,
    pub usb: Usb,
    pub log: Log,
    pub watchdog: WatchdogCfg,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct WatchdogCfg {
    /// the hardware watchdog the daemon feeds while healthy; "" = the daemon does not touch it
    pub device: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Paths {
    /// directory with policy.toml (default: the config file's directory)
    pub tables: Option<PathBuf>,
    /// the mode list the loaded sensor driver shows
    pub modes: PathBuf,
    /// the format list the loaded UVC gadget shows
    pub formats: PathBuf,
    pub iq_bin: PathBuf,
    /// the controls' settings file, written by microcamd alone
    pub settings: PathBuf,
}

/// How the sensor sits in the camera: its orientation as the vendor app set it, under the user's `mirror` and `flip`
/// controls.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Mounting {
    pub mirror: bool,
    pub flip: bool,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Control {
    pub unix_socket: PathBuf,
    /// TCP listen address for the laptop over the USB network; empty = none
    pub tcp: String,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Usb {
    /// the UVC gadget's V4L2 node
    pub device: PathBuf,
    pub enabled: bool,
    /// V4L2 buffers queued to the gadget (the vendor used 3; 2 lowered latency)
    pub buffers: u32,
    /// isochronous payload per transfer, bytes (`streaming_maxpacket` of g_webcam.ko)
    pub max_payload: u32,
    /// how long a STREAMON may wait for the pipeline and the first frame, ms
    pub streamon_budget_ms: u64,
    /// USB interface numbers of the UVC VideoControl and VideoStreaming interfaces in the composite gadget
    /// (g_webcam with RNDIS first: 2 and 3 — dmesg `uvcvideo 3-2:1.2` / `1.3`)
    pub vc_interface: u8,
    pub vs_interface: u8,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Log {
    /// as `microcam log-level` takes it: `debug`, `off`, `info,ui=debug,kernel=off`
    pub level: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            paths: Default::default(),
            mounting: Default::default(),
            control: Default::default(),
            usb: Default::default(),
            log: Default::default(),
            watchdog: Default::default(),
        }
    }
}
impl Default for Paths {
    fn default() -> Self {
        Self {
            tables: None,
            modes: "/sys/module/drv_ms_cus_gc8613_MIPI/parameters/modes".into(),
            formats: "/sys/module/g_webcam/parameters/formats".into(),
            iq_bin: "/customer/microcam/iq_file_gc8613.bin".into(),
            settings: "/customer/microcam/settings.json".into(),
        }
    }
}
impl Default for Mounting {
    fn default() -> Self {
        // the vendor app's `setmirrorflip 0 1 1`
        Self { mirror: true, flip: true }
    }
}
impl Default for Control {
    fn default() -> Self {
        Self { unix_socket: "/tmp/microcam.sock".into(), tcp: "0.0.0.0:7700".into() }
    }
}
impl Default for Usb {
    fn default() -> Self {
        Self { device: "/dev/video0".into(), enabled: true, buffers: 3, max_payload: microcam_core::heap::USB_PAYLOAD, streamon_budget_ms: 4000, vc_interface: 2, vs_interface: 3 }
    }
}
impl Default for WatchdogCfg {
    fn default() -> Self {
        Self { device: "/dev/watchdog0".into() }
    }
}
impl Default for Log {
    fn default() -> Self {
        Self { level: "debug".into() }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("config: {0}")]
    Parse(#[from] toml::de::Error),
}

impl Config {
    pub fn load(path: &Path) -> Result<(Self, PathBuf), ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|e| ConfigError::Io(path.to_path_buf(), e))?;
        let cfg: Config = toml::from_str(&text)?;
        let tables = cfg.paths.tables.clone().unwrap_or_else(|| path.parent().map(Path::to_path_buf).unwrap_or_else(|| ".".into()));
        Ok((cfg, tables))
    }

    /// The wishes at start: the controls' defaults, the mounting and the IQ file from here.
    pub fn prefs(&self) -> Prefs {
        Prefs { mirror: self.mounting.mirror, flip: self.mounting.flip, iq_bin: self.paths.iq_bin.to_string_lossy().into_owned(), ..Prefs::default() }
    }
}
