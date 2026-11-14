//! HDMI output timings and the monitor rule. The timing ids of the camera's `MI_HDMI_TimingType_e` / `MI_DISP` enums
//! belong to the `mi` crate; here a timing is a name with its size and rate.

use serde::{Deserialize, Serialize};

use crate::Size;

/// The HDMI timings this camera can output (HDMI 1.4 TX: 4K up to 30 Hz). Names as the user writes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Timing {
    #[serde(rename = "480p60")]
    P480_60,
    #[serde(rename = "576p50")]
    P576_50,
    #[serde(rename = "720p50")]
    P720_50,
    #[serde(rename = "720p60")]
    P720_60,
    #[serde(rename = "1080p24")]
    P1080_24,
    #[serde(rename = "1080p25")]
    P1080_25,
    #[serde(rename = "1080p30")]
    P1080_30,
    #[serde(rename = "1080p50")]
    P1080_50,
    #[serde(rename = "1080p60")]
    P1080_60,
    #[serde(rename = "1440p30")]
    P1440_30,
    #[serde(rename = "1440p50")]
    P1440_50,
    #[serde(rename = "1440p60")]
    P1440_60,
    #[serde(rename = "2160p25")]
    P2160_25,
    #[serde(rename = "2160p30")]
    P2160_30,
}

impl Timing {
    pub fn size(self) -> Size {
        use Timing::*;
        match self {
            P480_60 => Size::new(720, 480),
            P576_50 => Size::new(720, 576),
            P720_50 | P720_60 => Size::new(1280, 720),
            P1080_24 | P1080_25 | P1080_30 | P1080_50 | P1080_60 => Size::new(1920, 1080),
            P1440_30 | P1440_50 | P1440_60 => Size::new(2560, 1440),
            P2160_25 | P2160_30 => Size::new(3840, 2160),
        }
    }
    /// Lines in a frame, blanking included (CEA-861's vertical totals); None for a timing it does not list (1440p).
    pub fn v_total(self) -> Option<u32> {
        use Timing::*;
        match self {
            P480_60 => Some(525),
            P576_50 => Some(625),
            P720_50 | P720_60 => Some(750),
            P1080_24 | P1080_25 | P1080_30 | P1080_50 | P1080_60 => Some(1125),
            P2160_25 | P2160_30 => Some(2250),
            P1440_30 | P1440_50 | P1440_60 => None,
        }
    }
    pub fn rate_hz(self) -> u32 {
        use Timing::*;
        match self {
            P480_60 | P720_60 | P1080_60 | P1440_60 => 60,
            P576_50 | P720_50 | P1080_50 | P1440_50 => 50,
            P1080_30 | P1440_30 | P2160_30 => 30,
            P1080_25 | P2160_25 => 25,
            P1080_24 => 24,
        }
    }
    pub fn name(self) -> &'static str {
        use Timing::*;
        match self {
            P480_60 => "480p60",
            P576_50 => "576p50",
            P720_50 => "720p50",
            P720_60 => "720p60",
            P1080_24 => "1080p24",
            P1080_25 => "1080p25",
            P1080_30 => "1080p30",
            P1080_50 => "1080p50",
            P1080_60 => "1080p60",
            P1440_30 => "1440p30",
            P1440_50 => "1440p50",
            P1440_60 => "1440p60",
            P2160_25 => "2160p25",
            P2160_30 => "2160p30",
        }
    }
    pub fn parse(s: &str) -> Option<Timing> {
        serde_json_free_parse(s)
    }
}

fn serde_json_free_parse(s: &str) -> Option<Timing> {
    use Timing::*;
    Some(match s {
        "480p60" => P480_60,
        "576p50" => P576_50,
        "720p50" => P720_50,
        "720p60" => P720_60,
        "1080p24" => P1080_24,
        "1080p25" => P1080_25,
        "1080p30" => P1080_30,
        "1080p50" => P1080_50,
        "1080p60" => P1080_60,
        "1440p30" => P1440_30,
        "1440p50" => P1440_50,
        "1440p60" => P1440_60,
        "2160p25" => P2160_25,
        "2160p30" => P2160_30,
        _ => return None,
    })
}

/// What the monitor says it accepts, reduced to (size, rate) pairs. Built from the EDID by the daemon.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MonitorCaps {
    pub modes: Vec<(Size, u32)>,
}

impl MonitorCaps {
    pub fn supports(&self, t: Timing) -> bool {
        self.modes.iter().any(|&(s, r)| s == t.size() && r == t.rate_hz())
    }
}

/// Monitor state as the policy sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Monitor {
    Absent,
    /// connected, EDID read failed or empty
    PresentNoEdid,
    Present(MonitorCaps),
}

/// The default rule: the largest resolution the monitor offers among the camera's
/// candidate timings, and the highest rate there. `candidates` is the policy's ordered list (best first within a
/// resolution). Without an EDID: `no_edid`.
pub fn best_timing(monitor: &Monitor, candidates: &[Timing], no_edid: Timing) -> Option<Timing> {
    match monitor {
        Monitor::Absent => None,
        Monitor::PresentNoEdid => Some(no_edid),
        Monitor::Present(caps) => {
            let mut best: Option<Timing> = None;
            for &t in candidates {
                if !caps.supports(t) {
                    continue;
                }
                best = Some(match best {
                    None => t,
                    Some(b) => {
                        let (bs, ts) = (b.size(), t.size());
                        let (ba, ta) = (bs.w as u64 * bs.h as u64, ts.w as u64 * ts.h as u64);
                        if ta > ba || (ta == ba && t.rate_hz() > b.rate_hz()) {
                            t
                        } else {
                            b
                        }
                    }
                });
            }
            best.or(Some(no_edid))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn philips() -> MonitorCaps {
        // the Philips 276E8V offers 4K 24–60, 2560×1440@60, 1920×2160@60, 1080p50/60, 720p50/60, 480p, 576p
        let mut m = vec![];
        for r in [24, 25, 30, 50, 60] {
            m.push((Size::new(3840, 2160), r));
        }
        m.push((Size::new(2560, 1440), 60));
        m.push((Size::new(1920, 1080), 50));
        m.push((Size::new(1920, 1080), 60));
        m.push((Size::new(1280, 720), 60));
        MonitorCaps { modes: m }
    }

    fn candidates() -> Vec<Timing> {
        use Timing::*;
        vec![P2160_30, P2160_25, P1440_60, P1440_50, P1440_30, P1080_60, P1080_50, P1080_30, P1080_25, P1080_24, P720_60, P720_50, P576_50, P480_60]
    }

    #[test]
    fn philips_gets_2160p30_by_default() {
        let t = best_timing(&Monitor::Present(philips()), &candidates(), Timing::P1080_60);
        assert_eq!(t, Some(Timing::P2160_30));
    }

    #[test]
    fn a_1440p_monitor_gets_its_highest_rate() {
        let caps = MonitorCaps { modes: vec![(Size::new(2560, 1440), 60), (Size::new(2560, 1440), 30), (Size::new(1920, 1080), 60)] };
        assert_eq!(best_timing(&Monitor::Present(caps), &candidates(), Timing::P1080_60), Some(Timing::P1440_60));
    }

    #[test]
    fn no_monitor_none_no_edid_fallback() {
        assert_eq!(best_timing(&Monitor::Absent, &candidates(), Timing::P1080_60), None);
        assert_eq!(best_timing(&Monitor::PresentNoEdid, &candidates(), Timing::P1080_60), Some(Timing::P1080_60));
        let odd = MonitorCaps { modes: vec![(Size::new(1024, 768), 60)] };
        assert_eq!(best_timing(&Monitor::Present(odd), &candidates(), Timing::P1080_60), Some(Timing::P1080_60));
    }

    #[test]
    fn names_round_trip() {
        for t in candidates() {
            assert_eq!(Timing::parse(t.name()), Some(t));
        }
        assert_eq!(Timing::parse("2160p60"), None, "not a camera timing");
    }
}
