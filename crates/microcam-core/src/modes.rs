//! Sensor modes: what the driver exposes (`MI_SNR_GetRes`, read at start) plus the attributes only the driver's source
//! knows (line time, frame length): the mode list the loaded driver itself shows
//! (`/sys/module/drv_ms_cus_gc8613_MIPI/parameters/modes`), built into it from the same data as its register tables.
//! Identity is the mode **name**; driver indices are resolved at start against the live list.

use serde::Deserialize;
use std::collections::HashMap;

use crate::Size;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModeClass {
    /// the whole sensor at full resolution
    Full,
    /// 2×2 binned full field (1080p from the 4K sensor)
    Binned,
    /// a window of the sensor (narrower field of view)
    Crop,
}

/// One sensor mode as the driver's mode list describes it.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Mode {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub max_fps: u32,
    pub min_fps: u32,
    /// sensor line time in nanoseconds (sets the scaler budget: pixels per line on every enabled output)
    pub line_ns: u32,
    /// frame length in lines at max_fps
    pub frame_lines: u32,
    pub lanes: u32,
    pub class: ModeClass,
    /// true unless the driver's `fps_pinned = false` (mode 0): whether SetFPS is ignored by the driver
    #[serde(default = "default_true")]
    pub fps_pinned: bool,
}

fn default_true() -> bool {
    true
}

impl Mode {
    pub fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }
    /// Vertical blanking at max_fps, in microseconds. The ISP needs ~300–420 µs.
    pub fn vertical_gap_us(&self) -> f64 {
        (self.frame_lines.saturating_sub(self.height)) as f64 * self.line_ns as f64 / 1000.0
    }
    /// Frame length in lines for a frame rate at or below max_fps (the driver's `fps_live` rule:
    /// lines_per_second / fps, never shorter than the mode's own frame).
    pub fn frame_lines_at(&self, fps: u32) -> u32 {
        let lps = 1_000_000_000u64 / self.line_ns as u64;
        ((lps / fps.max(1) as u64) as u32).max(self.frame_lines)
    }
}

#[derive(Debug, Deserialize)]
struct ModesFile {
    schema: u32,
    #[serde(rename = "mode")]
    modes: Vec<Mode>,
}

/// A mode as the driver reports it live (`MI_SNR_GetRes`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DriverMode {
    pub index: u8,
    pub name: String,
    pub size: Size,
    pub max_fps: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum ModesError {
    #[error("{0}")]
    Parse(#[from] toml::de::Error),
    #[error("schema {0}, microcamd reads 1")]
    Schema(u32),
    #[error("mode '{0}' is in the driver's mode list but not in what the MI API lists")]
    MissingInDriver(String),
    #[error("duplicate mode name '{0}'")]
    Duplicate(String),
    #[error("no mode named '{0}'")]
    Unknown(String),
}

/// The validated mode table: the mode list's attributes joined with the driver's live indices.
#[derive(Clone, Debug)]
pub struct ModeTable {
    modes: Vec<Mode>,
    index_by_name: HashMap<String, (usize, u8)>,
}

impl ModeTable {
    /// Parse the driver's mode list without binding it to the MI list.
    pub fn parse(text: &str) -> Result<ParsedModes, ModesError> {
        let f: ModesFile = toml::from_str(text)?;
        if f.schema != 1 {
            return Err(ModesError::Schema(f.schema));
        }
        let mut seen = HashMap::new();
        for (i, m) in f.modes.iter().enumerate() {
            if seen.insert(m.name.clone(), i).is_some() {
                return Err(ModesError::Duplicate(m.name.clone()));
            }
        }
        Ok(ParsedModes { modes: f.modes })
    }

    /// Mode by name.
    pub fn get(&self, name: &str) -> Result<&Mode, ModesError> {
        self.index_by_name
            .get(name)
            .map(|&(i, _)| &self.modes[i])
            .ok_or_else(|| ModesError::Unknown(name.to_string()))
    }

    /// The driver's index of a mode (what `MI_SNR_SetRes` takes).
    pub fn driver_index(&self, name: &str) -> Result<u8, ModesError> {
        self.index_by_name.get(name).map(|&(_, d)| d).ok_or_else(|| ModesError::Unknown(name.to_string()))
    }

    pub fn iter(&self) -> impl Iterator<Item = &Mode> {
        self.modes.iter()
    }
}

/// A mode list parsed but not yet bound to the MI list.
#[derive(Clone, Debug)]
pub struct ParsedModes {
    pub modes: Vec<Mode>,
}

impl ParsedModes {
    /// Join with the MI API's live list of the same driver: every mode must be there by name, which gives the index the
    /// MI calls take (the MI list may carry more; policy cannot use those).
    pub fn bind(self, live: &[DriverMode]) -> Result<ModeTable, ModesError> {
        let mut index_by_name = HashMap::new();
        for (i, m) in self.modes.iter().enumerate() {
            let d = live
                .iter()
                .find(|d| d.name == m.name)
                .ok_or_else(|| ModesError::MissingInDriver(m.name.clone()))?;
            index_by_name.insert(m.name.clone(), (i, d.index));
        }
        Ok(ModeTable { modes: self.modes, index_by_name })
    }

    /// Whether the MI list is the stock driver's: that driver gives no mode list of its own and lists one mode,
    /// 3840x2160 at up to 30 fps.
    pub fn is_stock_driver(live: &[DriverMode]) -> bool {
        matches!(live, [d] if d.index == 0 && d.size == Size::new(3840, 2160) && d.max_fps == 30)
    }

    /// The driver's mode 0 alone, with the attributes the MI list carries and the stock 4K30 timing (14.8 µs line, 2250
    /// lines — the GalaxyCore stock table every GC8613 driver ships): all the stock driver has, and the conservative
    /// profile when a driver gives a mode list microcamd cannot use.
    pub fn conservative(live: &[DriverMode]) -> Option<ModeTable> {
        let d = live.iter().find(|d| d.index == 0)?;
        let m = Mode {
            name: d.name.clone(),
            width: d.size.w,
            height: d.size.h,
            max_fps: d.max_fps,
            min_fps: 5,
            line_ns: 14814,
            frame_lines: 2250,
            lanes: 4,
            class: ModeClass::Full,
            fps_pinned: false,
        };
        let mut index_by_name = HashMap::new();
        index_by_name.insert(m.name.clone(), (0usize, 0u8));
        Some(ModeTable { modes: vec![m], index_by_name })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The mode list the driver is built with and shows (kmods/gc8613/modes.py).
    pub(crate) fn modes_toml() -> &'static str {
        static T: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        T.get_or_init(|| {
            let py = concat!(env!("CARGO_MANIFEST_DIR"), "/../../kmods/gc8613/modes.py");
            let out = std::process::Command::new("python3").args([py, "list"]).output().expect("python3");
            assert!(out.status.success(), "modes.py: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8(out.stdout).unwrap()
        })
    }

    pub(crate) fn live_from_file(p: &ParsedModes) -> Vec<DriverMode> {
        p.modes
            .iter()
            .enumerate()
            .map(|(i, m)| DriverMode { index: i as u8, name: m.name.clone(), size: m.size(), max_fps: m.max_fps })
            .collect()
    }

    pub(crate) fn table() -> ModeTable {
        let p = ModeTable::parse(modes_toml()).unwrap();
        let live = live_from_file(&p);
        p.bind(&live).unwrap()
    }

    #[test]
    fn parses_the_driver_s_list() {
        let t = table();
        assert_eq!(t.iter().count(), 14);
        let m = t.get("3840x2160@45 hts340").unwrap();
        assert_eq!((m.width, m.height, m.max_fps, m.line_ns), (3840, 2160, 45, 9481));
        assert_eq!(t.driver_index("3840x2160@45 hts340").unwrap(), 1);
        assert_eq!(t.driver_index("3840x2160@40 rk fastpll").unwrap(), 3);
        assert!(!t.get("3840x2160@30 stock").unwrap().fps_pinned);
    }

    #[test]
    fn vertical_gap_and_frame_length() {
        let t = table();
        let m = t.get("3840x2160@30 stock").unwrap();
        assert!((m.vertical_gap_us() - 1333.3).abs() < 0.1);
        assert_eq!(m.frame_lines_at(30), 2250);
        assert_eq!(m.frame_lines_at(60), 2250, "faster than the mode: never shorter than its own frame");
        let m = t.get("1920x1080@121 rk40 bin hts278").unwrap();
        assert_eq!(m.frame_lines_at(60), 2314); // 1e9/7200 ns = 138 888 lines/s ÷ 60
    }

    #[test]
    fn a_list_of_another_schema_is_refused() {
        let text = modes_toml().replacen("schema = 1", "schema = 2", 1);
        assert!(matches!(ModeTable::parse(&text), Err(ModesError::Schema(2))));
    }

    #[test]
    fn conservative_profile_has_only_mode_0() {
        let p = ModeTable::parse(modes_toml()).unwrap();
        let live = live_from_file(&p);
        let t = ParsedModes::conservative(&live).unwrap();
        assert_eq!(t.iter().count(), 1);
        assert_eq!(t.driver_index("3840x2160@30 stock").unwrap(), 0);
        assert!(!ParsedModes::is_stock_driver(&live));
        let stock = [DriverMode { index: 0, name: String::new(), size: Size::new(3840, 2160), max_fps: 30 }];
        assert!(ParsedModes::is_stock_driver(&stock));
        assert_eq!(ParsedModes::conservative(&stock).unwrap().iter().count(), 1);
    }
}
