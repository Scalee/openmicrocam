//! USB streaming formats: the format list the loaded gadget itself shows (`/sys/module/g_webcam/parameters/formats`),
//! built into it from the descriptors it advertises. UVC probe/commit name a format and a frame by **index**, so the list
//! order is the descriptor order. A gadget that shows none is taken for the stock one, whose list microcamd carries
//! ([`STOCK_GADGET`]).

use serde::Deserialize;

use crate::{Codec, Size};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Frame {
    pub index: u8,
    pub width: u32,
    pub height: u32,
    /// frame intervals in UVC units (100 ns), as advertised, fastest first
    pub intervals: Vec<u32>,
    pub default_interval: u32,
}

impl Frame {
    pub fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }
    /// The interval the host's request maps to: the request itself within the advertised range (the camera runs any rate
    /// up to the mode's ceiling, and answers a probe with the rate it will deliver, which may lie between the advertised
    /// ones), else the nearest end of the range. 0 = "use the default".
    pub fn match_interval(&self, requested: u32) -> u32 {
        if requested == 0 {
            return self.default_interval;
        }
        let fastest = self.intervals.iter().copied().min().unwrap_or(self.default_interval);
        let slowest = self.intervals.iter().copied().max().unwrap_or(self.default_interval);
        requested.clamp(fastest, slowest)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Format {
    pub index: u8,
    pub codec: Codec,
    pub default_frame: u8,
    #[serde(rename = "frame", default)]
    pub frames: Vec<Frame>,
}

#[derive(Debug, Deserialize)]
struct FormatsFile {
    schema: u32,
    #[serde(rename = "format")]
    formats: Vec<Format>,
}

#[derive(Debug, thiserror::Error)]
pub enum FormatsError {
    #[error("{0}")]
    Parse(#[from] toml::de::Error),
    #[error("schema {0}, microcamd reads 1")]
    Schema(u32),
    #[error("format index {0} out of range")]
    NoFormat(u8),
    #[error("frame index {1} out of range for format {0}")]
    NoFrame(u8, u8),
}

/// The stock gadget's streaming formats, a second copy of what that module holds: the module gives microcamd no list, so
/// these facts are carried here. They are the descriptors of the YIZHAN 4K firmware's own `g_webcam.ko` (the file
/// `tools/install/stock-parts.tsv` names, sha256 f719a706…), which the kernel shows as its `srcversion`; the host test
/// `stock_list_is_the_stock_module_s` compares them with that file whenever the camera's dump is at hand.
pub struct StockGadget {
    /// `/sys/module/g_webcam/srcversion` of the module these facts are about
    pub srcversion: &'static str,
    /// format 1, 2, 3: codec and default frame index
    pub formats: [(Codec, u8); 3],
    /// each format's frames in index order (frame 1 first)
    pub sizes: [[(u32, u32); 7]; 3],
    /// every frame's discrete intervals (100 ns units), fastest first, and its default
    pub intervals: [u32; 3],
    pub default_interval: u32,
}

const FB_SIZES: [(u32, u32); 7] = [(320, 240), (640, 480), (1280, 720), (1920, 1080), (2560, 1440), (3840, 2160), (424, 240)];

pub const STOCK_GADGET: StockGadget = StockGadget {
    srcversion: "AD2AE2F37C66E50AC54B7BF",
    formats: [(Codec::Mjpeg, 1), (Codec::H264, 1), (Codec::H265, 1)],
    sizes: [[(3840, 2160), (640, 480), (1280, 720), (1920, 1080), (2560, 1440), (320, 240), (424, 240)], FB_SIZES, FB_SIZES],
    intervals: [333_333, 666_666, 1_000_000],
    default_interval: 333_333,
};

/// What a host asked for (or committed), already resolved against the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostStream {
    pub codec: Codec,
    pub size: Size,
    /// frames per second as the camera will run them (interval clamped to the advertised range)
    pub fps: u32,
    pub format_index: u8,
    pub frame_index: u8,
    pub interval: u32,
}

#[derive(Clone, Debug)]
pub struct FormatTable {
    formats: Vec<Format>,
}

impl FormatTable {
    pub fn parse(text: &str) -> Result<Self, FormatsError> {
        let f: FormatsFile = toml::from_str(text)?;
        if f.schema != 1 {
            return Err(FormatsError::Schema(f.schema));
        }
        Ok(Self { formats: f.formats })
    }

    /// The stock gadget's list, which that module cannot show: its streaming descriptors as [`STOCK_GADGET`] records them.
    pub fn stock() -> Self {
        let formats = STOCK_GADGET
            .formats
            .iter()
            .enumerate()
            .map(|(i, &(codec, default_frame))| Format {
                index: i as u8 + 1,
                codec,
                default_frame,
                frames: STOCK_GADGET.sizes[i]
                    .iter()
                    .enumerate()
                    .map(|(j, &(width, height))| Frame {
                        index: j as u8 + 1,
                        width,
                        height,
                        intervals: STOCK_GADGET.intervals.to_vec(),
                        default_interval: STOCK_GADGET.default_interval,
                    })
                    .collect(),
            })
            .collect();
        Self { formats }
    }

    /// When a gadget gives a list microcamd cannot use: only the stock list's first format and frame at its default rate
    /// (MJPEG 3840x2160@30, format 1, frame 1), which every build of the gadget keeps at those indices. Any other proposal
    /// is answered with it.
    pub fn conservative() -> Self {
        let mut format = Self::stock().formats.swap_remove(0);
        format.frames.truncate(1);
        format.frames[0].intervals = vec![format.frames[0].default_interval];
        Self { formats: vec![format] }
    }

    pub fn format(&self, index: u8) -> Result<&Format, FormatsError> {
        self.formats.iter().find(|f| f.index == index).ok_or(FormatsError::NoFormat(index))
    }

    pub fn frame(&self, format: u8, frame: u8) -> Result<&Frame, FormatsError> {
        self.format(format)?.frames.iter().find(|f| f.index == frame).ok_or(FormatsError::NoFrame(format, frame))
    }

    /// Resolve a probe/commit (format index, frame index, requested interval in 100 ns; 0 = default) to the stream
    /// the camera will run.
    pub fn resolve(&self, format: u8, frame: u8, interval: u32) -> Result<HostStream, FormatsError> {
        let fmt = self.format(format)?;
        let fr = self.frame(format, frame)?;
        let iv = fr.match_interval(interval);
        Ok(HostStream {
            codec: fmt.codec,
            size: fr.size(),
            fps: ((10_000_000.0 / iv as f64).round() as u32).max(1),
            format_index: format,
            frame_index: frame,
            interval: iv,
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = &Format> {
        self.formats.iter()
    }

    /// The format and frame a host gets when it asks for none: the first format, its descriptor's default frame (0 names
    /// none: its first frame).
    pub fn default(&self) -> (u8, u8) {
        self.formats.first().map_or((1, 1), |f| (f.index, f.default_frame.max(1)))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The format list the gadget is built with and shows: the C string `yz_format_list` gen_desc.py writes into
    /// kmods/g_webcam/yz_desc.h from the descriptors of the dump's stock module. The tests do not compile without it.
    pub(crate) fn formats_toml() -> &'static str {
        static T: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        T.get_or_init(|| {
            let h = include_str!("../../../kmods/g_webcam/yz_desc.h"); // made by kmods/g_webcam/build.sh (README «Building»)
            let list = h.split_once("yz_format_list[] =\n").expect("yz_format_list in yz_desc.h").1;
            let list = list.split_once(";\n").expect("the list's end").0;
            list.lines().map(|l| l.trim().trim_start_matches('"').trim_end_matches("\\n\"").replace("\\\"", "\"") + "\n").collect()
        })
    }
    /// The built gadget's format indices (`kmods/g_webcam/gen_desc.py`).
    pub(crate) const MJPEG: u8 = 1;
    pub(crate) const H264: u8 = 2;
    pub(crate) const H265: u8 = 3;

    pub(crate) fn table() -> FormatTable {
        FormatTable::parse(formats_toml()).unwrap()
    }

    #[test]
    fn a_list_of_another_schema_is_refused() {
        let text = formats_toml().replacen("schema = 1", "schema = 2", 1);
        assert!(matches!(FormatTable::parse(&text), Err(FormatsError::Schema(2))));
    }

    #[test]
    fn conservative_profile_answers_every_proposal_with_mjpeg_4k30() {
        let t = FormatTable::conservative();
        assert_eq!(t.iter().count(), 1);
        assert_eq!(t.format(MJPEG).unwrap().frames.len(), 1);
        assert_eq!(t.default(), (MJPEG, 1));
        let s = t.resolve(MJPEG, 1, 0).unwrap();
        assert_eq!((s.codec, s.size, s.fps), (Codec::Mjpeg, Size::new(3840, 2160), 30));
        assert_eq!(t.resolve(MJPEG, 1, 222_222).unwrap().fps, 30);
        assert_eq!(t.resolve(MJPEG, 1, 1_000_000).unwrap().fps, 30);
    }

    #[test]
    fn the_stock_list_has_three_formats_of_seven_frames_at_30_15_10() {
        let t = FormatTable::stock();
        assert_eq!(t.iter().map(|f| f.codec).collect::<Vec<_>>(), [Codec::Mjpeg, Codec::H264, Codec::H265]);
        assert!(t.iter().all(|f| f.frames.len() == 7));
        assert_eq!(t.frame(MJPEG, 1).unwrap().size(), Size::new(3840, 2160));
        assert_eq!(t.frame(H264, 6).unwrap().size(), Size::new(3840, 2160));
        assert_eq!(t.frame(H265, 1).unwrap().size(), Size::new(320, 240));
        assert_eq!(t.default(), (MJPEG, 1));
        assert_eq!(t.resolve(MJPEG, 1, 0).unwrap().fps, 30);
        assert_eq!(t.resolve(MJPEG, 1, 222_222).unwrap().fps, 30);
        assert_eq!(t.resolve(H264, 2, 666_666).unwrap().fps, 15);
        assert_eq!(t.resolve(H265, 7, 1_000_000).unwrap().fps, 10);
    }

    /// The stock gadget's streaming list as its module file holds it: the pointer array of its streaming descriptors in
    /// `.data` (at 0x1b4, each entry the `.data` offset of a descriptor), walked as the gadget hands it to the UVC
    /// function. (formats, srcversion)
    fn stock_module_list(ko: &[u8]) -> (Vec<Format>, String) {
        let u16_at = |o: usize| u16::from_le_bytes([ko[o], ko[o + 1]]);
        let u32_at = |o: usize| u32::from_le_bytes(ko[o..o + 4].try_into().unwrap());
        let (shoff, shentsize, shnum, shstrndx) = (u32_at(0x20) as usize, u16_at(0x2e) as usize, u16_at(0x30) as usize, u16_at(0x32) as usize);
        let sec = |i: usize| (u32_at(shoff + i * shentsize) as usize, u32_at(shoff + i * shentsize + 16) as usize, u32_at(shoff + i * shentsize + 20) as usize);
        let names = sec(shstrndx).1;
        let section = |want: &str| {
            (0..shnum).map(sec).find(|&(n, _, _)| ko[names + n..].split(|&c| c == 0).next() == Some(want.as_bytes())).map(|(_, o, s)| &ko[o..o + s]).unwrap()
        };
        let data = section(".data");
        let srcversion = section(".modinfo")
            .split(|&c| c == 0)
            .find_map(|kv| kv.strip_prefix(b"srcversion="))
            .map(|v| String::from_utf8_lossy(v).into_owned())
            .unwrap_or_default();
        let guid = |g: &[u8]| match &g[..4] {
            b"H264" => Codec::H264,
            b"H265" => Codec::H265,
            other => panic!("format GUID {other:?}"),
        };
        let mut formats: Vec<Format> = Vec::new();
        for k in 0.. {
            let at = u32::from_le_bytes(data[0x1b4 + 4 * k..0x1b8 + 4 * k].try_into().unwrap()) as usize;
            if at == 0 {
                break;
            }
            let x = &data[at..at + data[at] as usize];
            assert_eq!(x[1], 0x24, "a class-specific descriptor at .data {at:#x}");
            let le32 = |o: usize| u32::from_le_bytes(x[o..o + 4].try_into().unwrap());
            match x[2] {
                0x06 => formats.push(Format { index: x[3], codec: Codec::Mjpeg, default_frame: x[6], frames: vec![] }),
                0x10 => formats.push(Format { index: x[3], codec: guid(&x[5..21]), default_frame: x[22], frames: vec![] }),
                0x07 | 0x11 => {
                    let (n, dflt) = if x[2] == 0x07 { (x[25] as usize, le32(21)) } else { (x[21] as usize, le32(17)) };
                    formats.last_mut().unwrap().frames.push(Frame {
                        index: x[3],
                        width: u16::from_le_bytes([x[5], x[6]]) as u32,
                        height: u16::from_le_bytes([x[7], x[8]]) as u32,
                        intervals: (0..n).map(|i| le32(26 + 4 * i)).collect(),
                        default_interval: dflt,
                    });
                }
                _ => {} // the input header, the stills, the colour matching
            }
        }
        (formats, srcversion)
    }

    /// The carried stock list against the stock module of the camera's dump (`MICROCAM_SYSROOT` as tools/hostconf.py has
    /// it, docs/TOOLS.md); without a dump there is nothing to compare and the test says so.
    #[test]
    fn stock_list_is_the_stock_module_s() {
        let root = env!("CARGO_MANIFEST_DIR");
        let sysroot = std::process::Command::new("python3").arg(format!("{root}/../../tools/hostconf.py")).output().ok()
            .and_then(|out| String::from_utf8(out.stdout).ok())
            .and_then(|text| text.lines().find_map(|l| l.strip_prefix("MICROCAM_SYSROOT=").map(str::to_string)))
            .filter(|s| std::path::Path::new(s).is_dir());
        let Some(sysroot) = sysroot else {
            eprintln!("no camera dump (MICROCAM_SYSROOT): the stock list is not compared");
            return;
        };
        let ko = std::fs::read(format!("{sysroot}/miservice/modules/4.9.227/g_webcam.ko")).expect("the dump's stock g_webcam.ko");
        let (formats, srcversion) = stock_module_list(&ko);
        assert_eq!(srcversion, STOCK_GADGET.srcversion);
        assert_eq!(formats, FormatTable::stock().formats);
    }

    #[test]
    fn the_gadget_s_list_has_three_formats_of_seven_frames() {
        let t = table();
        assert_eq!(t.iter().count(), 3);
        assert!(t.iter().all(|f| f.frames.len() == 7));
        assert_eq!(t.format(MJPEG).unwrap().codec, Codec::Mjpeg);
        assert_eq!(t.format(H264).unwrap().codec, Codec::H264);
        assert_eq!(t.format(H265).unwrap().codec, Codec::H265);
        assert_eq!(t.frame(MJPEG, 1).unwrap().size(), Size::new(3840, 2160));
        assert_eq!(t.frame(MJPEG, 4).unwrap().size(), Size::new(1920, 1080));
        assert_eq!(t.frame(H264, 6).unwrap().size(), Size::new(3840, 2160));
        // a host taking the first format and its default frame and interval gets MJPEG 4K45
        let (f, fr) = t.default();
        assert_eq!(t.format(f).unwrap().codec, Codec::Mjpeg);
        assert_eq!(t.frame(f, fr).unwrap().size(), Size::new(3840, 2160));
        assert_eq!(t.resolve(f, fr, 0).unwrap().fps, 45);
    }

    #[test]
    fn interval_matching_clamps_to_the_advertised_range() {
        let t = table();
        // 1080p MJPEG advertises 120/60/30; the host's 240 fps request (41666) → 120 fps; 30 fps → 30; 0 → default 30
        let s = t.resolve(MJPEG, 4, 41_666).unwrap();
        assert_eq!((s.fps, s.interval), (120, 83_333));
        assert_eq!(t.resolve(MJPEG, 4, 333_333).unwrap().fps, 30);
        assert_eq!(t.resolve(MJPEG, 4, 0).unwrap().fps, 30);
        // a rate between the advertised ones stays as asked (the camera's own 40-fps probe answer comes back in the commit)
        let s = t.resolve(MJPEG, 4, 250_000).unwrap();
        assert_eq!((s.fps, s.interval), (40, 250_000));
        // a request slower than everything advertised gets the slowest
        assert_eq!(t.resolve(MJPEG, 1, 2_000_000).unwrap().fps, 10);
        // 4K MJPEG at 45
        assert_eq!(t.resolve(MJPEG, 1, 222_222).unwrap().fps, 45);
        assert!(matches!(t.resolve(MJPEG, 9, 0), Err(FormatsError::NoFrame(MJPEG, 9))));
    }
}
