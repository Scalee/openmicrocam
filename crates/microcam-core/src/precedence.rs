//! Who gives way when the camera is short: every holder of its resources once, the most important first, each
//! with an optional floor — the lowest of its steps it may go to. The setting `precedence` holds it; microcamd decides by
//! it at once, for memory ([`Precedence::give_way`]) and for the two limits that are not memory (the policy: which output
//! pauses beside a recording, which one gives way on the scaler they share).

use serde::{Deserialize, Serialize};

use crate::hdmi::Timing;

/// A holder of the camera's resources: a place in the precedence, and what a refusal for lack of them names
/// ([`crate::controls::Held`] carries one with what it holds).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Who {
    Recording,
    Player,
    Usb,
    Hdmi,
    /// a photo being taken (its media chain)
    Photo,
    /// microcamd's share less the pictures: the menu's tiles, windows, bands, patches
    Drawing,
    /// the menu's full-colour pictures: a frozen frame, the compare, the gallery's photos
    Picture,
}

impl Who {
    pub const ALL: [Who; 7] = [Who::Recording, Who::Player, Who::Usb, Who::Hdmi, Who::Photo, Who::Drawing, Who::Picture];

    /// Its place in the default precedence: microcamd's decisions as they were before the setting (USB
    /// first, one output beside a recording, the video before the menu, a photo given up for a recording, a recording before a freeze), the
    /// menu's pictures before the menu itself. No wildcard: a holder added fails the build here until it has a place.
    fn default_place(self) -> usize {
        match self {
            Who::Recording => 0,
            Who::Player => 1,
            Who::Usb => 2,
            Who::Hdmi => 3,
            Who::Photo => 4,
            Who::Drawing => 5,
            Who::Picture => 6,
        }
    }

    /// What it can give, best first, by the words a floor names; a recording has none: a running one is never cut, one
    /// not started is only refused.
    pub fn steps(self) -> &'static [&'static str] {
        match self {
            Who::Hdmi => &["lower", "paused", "off"],
            Who::Usb => &["lower", "paused", "refused"],
            Who::Photo => &["given-up"],
            Who::Player => &["stopped"],
            Who::Picture => &["not-shown"],
            Who::Drawing => &["off"],
            Who::Recording => &[],
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Who::Recording => "recording",
            Who::Player => "player",
            Who::Usb => "usb",
            Who::Hdmi => "hdmi",
            Who::Photo => "photo",
            Who::Drawing => "drawing",
            Who::Picture => "picture",
        }
    }

    fn parse(s: &str) -> Option<Who> {
        Who::ALL.into_iter().find(|w| w.name() == s)
    }
}

/// One step a holder gives: HDMI a lower timing, paused (its frame without the live picture) or off; USB paused or
/// refused (no frames); a photo given up; the player stopped; the menu's pictures not shown; its drawing off the
/// outputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Step {
    Timing(Timing),
    Paused,
    Off,
    Refused,
    GivenUp,
    Stopped,
    NotShown,
}

impl Step {
    /// The step's word among its holder's [`Who::steps`].
    pub fn word(self) -> &'static str {
        match self {
            Step::Timing(_) => "lower",
            Step::Paused => "paused",
            Step::Off => "off",
            Step::Refused => "refused",
            Step::GivenUp => "given-up",
            Step::Stopped => "stopped",
            Step::NotShown => "not-shown",
        }
    }
}

/// A holder's place: who, and the deepest of its steps it may go to (an index into [`Who::steps`]; None: all).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub who: Who,
    pub floor: Option<usize>,
}

/// The precedence: every holder once, the most important first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Precedence(Vec<Place>);

impl Default for Precedence {
    fn default() -> Self {
        let mut v: Vec<Place> = Who::ALL.iter().map(|&who| Place { who, floor: None }).collect();
        v.sort_by_key(|p| p.who.default_place());
        Self(v)
    }
}

impl std::fmt::Display for Precedence {
    /// `recording > player > usb > hdmi:paused > …`: a floor after its holder.
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        for (i, p) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(" > ")?;
            }
            f.write_str(p.who.name())?;
            if let Some(k) = p.floor {
                write!(f, ":{}", p.who.steps()[k])?;
            }
        }
        Ok(())
    }
}

impl Precedence {
    /// The precedence `s` names, checked whole: every holder once, each floor one of its holder's steps.
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut v: Vec<Place> = Vec::new();
        for part in s.split('>').map(str::trim) {
            let (name, floor) = match part.split_once(':') {
                Some((n, f)) => (n.trim(), Some(f.trim())),
                None => (part, None),
            };
            let who = Who::parse(name).ok_or_else(|| format!("'{name}' is no holder (holders: {})", Who::ALL.map(Who::name).join(", ")))?;
            if v.iter().any(|p| p.who == who) {
                return Err(format!("{name} named twice"));
            }
            let floor = match floor {
                None => None,
                Some(f) => Some(who.steps().iter().position(|s| *s == f).ok_or_else(|| match who.steps() {
                    [] => format!("{name} has no step to set a floor at"),
                    steps => format!("'{f}' is not a step of {name} ({})", steps.join(", ")),
                })?),
            };
            v.push(Place { who, floor });
        }
        if let Some(w) = Who::ALL.into_iter().find(|w| !v.iter().any(|p| p.who == *w)) {
            return Err(format!("{} is not named: every holder takes a place", w.name()));
        }
        Ok(Self(v))
    }

    pub fn places(&self) -> &[Place] {
        &self.0
    }

    fn rank(&self, w: Who) -> usize {
        self.0.iter().position(|p| p.who == w).expect("every holder has a place")
    }

    /// Whether `a` comes before `b`.
    pub fn above(&self, a: Who, b: Who) -> bool {
        self.rank(a) < self.rank(b)
    }

    /// Whether `w` may give `step`: not below its floor.
    pub fn allows(&self, w: Who, step: Step) -> bool {
        let Some(k) = w.steps().iter().position(|s| *s == step.word()) else { return false };
        self.0[self.rank(w)].floor.is_none_or(|f| k <= f)
    }

    /// The same order with `a` and `b` in each other's places when `b` is above `a` (each keeps its floor): `a` first
    /// of the two.
    pub fn with_above(&self, a: Who, b: Who) -> Self {
        let mut v = self.0.clone();
        if !self.above(a, b) {
            v.swap(self.rank(a), self.rank(b));
        }
        Self(v)
    }

    /// Who gives way, and with which step, for `asker` short of memory: from the bottom of the precedence up to the
    /// asker's place, the first holder one of whose steps fits gives way at the smallest such step; when none fits
    /// alone, the holders from the bottom up together, each but the last at the deepest step its floor allows. None
    /// when nothing below the asker can: the asker is refused. A holder above the asker never gives way to it.
    /// `ladders`: each holder that holds something now with the steps it can give from where it stands, best first;
    /// `fits` prices a set of steps by the code that would build them.
    pub fn give_way(&self, asker: Who, ladders: &[(Who, Vec<Step>)], fits: impl FnMut(&[(Who, Step)]) -> bool) -> Option<Vec<(Who, Step)>> {
        self.walk(asker, None, ladders, fits)
    }

    /// [`Self::give_way`] for an asker that may give itself up: a paused output whose frame does not fit asks for it,
    /// and its `own` step — HDMI off, USB refused — is the walk's top rung, above every holder below it and not
    /// below its floor.
    pub fn give_way_or_go(&self, asker: Who, own: Step, ladders: &[(Who, Vec<Step>)], fits: impl FnMut(&[(Who, Step)]) -> bool) -> Option<Vec<(Who, Step)>> {
        self.walk(asker, Some(own), ladders, fits)
    }

    fn walk(&self, asker: Who, own: Option<Step>, ladders: &[(Who, Vec<Step>)], mut fits: impl FnMut(&[(Who, Step)]) -> bool) -> Option<Vec<(Who, Step)>> {
        let mut below: Vec<(Who, Vec<Step>)> = self.0[self.rank(asker) + 1..]
            .iter()
            .rev()
            .filter_map(|p| ladders.iter().find(|(w, _)| *w == p.who))
            .map(|(w, steps)| (*w, steps.iter().copied().filter(|s| self.allows(*w, *s)).collect::<Vec<_>>()))
            .filter(|(_, steps)| !steps.is_empty())
            .collect();
        below.extend(own.filter(|s| self.allows(asker, *s)).map(|s| (asker, vec![s])));
        for (w, steps) in &below {
            if let Some(s) = steps.iter().find(|s| fits(&[(*w, **s)])) {
                return Some(vec![(*w, *s)]);
            }
        }
        let mut all: Vec<(Who, Step)> = Vec::new();
        for (w, steps) in &below {
            for s in steps {
                all.push((*w, *s));
                if fits(&all) {
                    return Some(all);
                }
                all.pop();
            }
            all.push((*w, *steps.last().expect("not empty")));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_the_order_microcamd_decided_by_and_reads_back() {
        let d = Precedence::default();
        assert_eq!(d.to_string(), "recording > player > usb > hdmi > photo > drawing > picture");
        assert_eq!(Precedence::parse(&d.to_string()), Ok(d));
        // every holder once, each a place of its own
        let mut places: Vec<usize> = Who::ALL.map(Who::default_place).to_vec();
        places.sort_unstable();
        assert_eq!(places, (0..Who::ALL.len()).collect::<Vec<_>>());
    }

    #[test]
    fn a_queue_is_checked_whole() {
        let ok = Precedence::parse(" recording>player > hdmi:paused > usb > photo > drawing > picture").unwrap();
        assert_eq!(ok.to_string(), "recording > player > hdmi:paused > usb > photo > drawing > picture");
        for bad in [
            "recording > player > usb > hdmi > photo > drawing",
            "recording > player > usb > hdmi > photo > drawing > picture > usb",
            "recording > player > usb > hdmi > photo > drawing > picture > tv",
            "recording > player > usb > hdmi:lowest > photo > drawing > picture",
            "recording:off > player > usb > hdmi > photo > drawing > picture",
            "",
        ] {
            assert!(Precedence::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_floor_bounds_the_steps() {
        let p = Precedence::parse("recording > player > usb > hdmi:paused > photo > drawing > picture").unwrap();
        assert!(p.allows(Who::Hdmi, Step::Timing(Timing::P1080_60)));
        assert!(p.allows(Who::Hdmi, Step::Paused));
        assert!(!p.allows(Who::Hdmi, Step::Off));
        assert!(p.allows(Who::Usb, Step::Refused));
        assert!(!p.allows(Who::Usb, Step::Off), "not a step of USB");
    }

    #[test]
    fn swapping_two_keeps_their_floors() {
        let p = Precedence::parse("recording > player > usb:paused > hdmi > photo > drawing > picture").unwrap();
        assert_eq!(p.with_above(Who::Hdmi, Who::Usb).to_string(), "recording > player > hdmi > usb:paused > photo > drawing > picture");
        assert_eq!(p.with_above(Who::Usb, Who::Hdmi), p);
    }

    /// Prices: what each step frees; `fits` when they free `x`.
    fn by(prices: &[(Who, Step, u64)], x: u64) -> impl FnMut(&[(Who, Step)]) -> bool + '_ {
        move |set: &[(Who, Step)]| set.iter().map(|(w, s)| prices.iter().find(|p| (p.0, p.1) == (*w, *s)).map_or(0, |p| p.2)).sum::<u64>() >= x
    }

    fn ladders() -> Vec<(Who, Vec<Step>)> {
        vec![
            (Who::Picture, vec![Step::NotShown]),
            (Who::Drawing, vec![Step::Off]),
            (Who::Hdmi, vec![Step::Timing(Timing::P1440_60), Step::Timing(Timing::P1080_60), Step::Paused, Step::Off]),
            (Who::Recording, vec![]),
        ]
    }

    const PRICES: [(Who, Step, u64); 6] = [
        (Who::Picture, Step::NotShown, 12),
        (Who::Drawing, Step::Off, 20),
        (Who::Hdmi, Step::Timing(Timing::P1440_60), 30),
        (Who::Hdmi, Step::Timing(Timing::P1080_60), 40),
        (Who::Hdmi, Step::Paused, 40),
        (Who::Hdmi, Step::Off, 60),
    ];

    #[test]
    fn the_lowest_that_frees_it_alone_gives_way_at_its_smallest_step() {
        let p = Precedence::default();
        // the pictures before the menu itself
        assert_eq!(p.give_way(Who::Usb, &ladders(), by(&PRICES, 10)), Some(vec![(Who::Picture, Step::NotShown)]));
        assert_eq!(p.give_way(Who::Usb, &ladders(), by(&PRICES, 15)), Some(vec![(Who::Drawing, Step::Off)]));
        assert_eq!(p.give_way(Who::Usb, &ladders(), by(&PRICES, 35)), Some(vec![(Who::Hdmi, Step::Timing(Timing::P1080_60))]));
    }

    #[test]
    fn none_alone_the_holders_from_the_bottom_up_together() {
        let p = Precedence::default();
        assert_eq!(p.give_way(Who::Usb, &ladders(), by(&PRICES, 70)), Some(vec![(Who::Picture, Step::NotShown), (Who::Drawing, Step::Off), (Who::Hdmi, Step::Timing(Timing::P1080_60))]));
        assert_eq!(p.give_way(Who::Usb, &ladders(), by(&PRICES, 93)), None, "not enough below the asker");
    }

    #[test]
    fn a_holder_above_the_asker_never_gives_way_to_it() {
        let p = Precedence::default();
        // HDMI asks: only the photo, the drawing and the pictures are below it
        assert_eq!(p.give_way(Who::Hdmi, &ladders(), by(&PRICES, 35)), None);
        assert_eq!(p.give_way(Who::Picture, &ladders(), by(&PRICES, 1)), None, "nothing below the last");
    }

    #[test]
    fn a_floor_is_respected() {
        let off = vec![(Who::Picture, Step::NotShown), (Who::Drawing, Step::Off), (Who::Hdmi, Step::Off)];
        assert_eq!(Precedence::default().give_way(Who::Usb, &ladders(), by(&PRICES, 73)), Some(off));
        // HDMI off would free it, its floor allows only lower timings: refused
        let p = Precedence::parse("recording > player > usb > hdmi:lower > photo > drawing > picture").unwrap();
        assert_eq!(p.give_way(Who::Usb, &ladders(), by(&PRICES, 73)), None);
        assert_eq!(p.give_way(Who::Usb, &ladders(), by(&PRICES, 72)), Some(vec![(Who::Picture, Step::NotShown), (Who::Drawing, Step::Off), (Who::Hdmi, Step::Timing(Timing::P1080_60))]));
        // paused frees no more than the lowest timing here: still refused
        let p = Precedence::parse("recording > player > usb > hdmi:paused > photo > drawing > picture").unwrap();
        assert_eq!(p.give_way(Who::Usb, &ladders(), by(&PRICES, 73)), None);
        assert_eq!(p.give_way(Who::Usb, &ladders(), by(&PRICES, 50)), Some(vec![(Who::Picture, Step::NotShown), (Who::Drawing, Step::Off), (Who::Hdmi, Step::Timing(Timing::P1440_60))]));
    }

    /// HDMI paused beside a recording, its frame short of the heap: what each step frees, HDMI off its frame.
    const PAUSED: [(Who, Step, u64); 3] = [(Who::Picture, Step::NotShown, 12), (Who::Drawing, Step::Off, 20), (Who::Hdmi, Step::Off, 36)];

    #[test]
    fn a_paused_output_goes_dark_only_when_nothing_below_it_frees_enough() {
        let p = Precedence::default();
        let ladders = vec![(Who::Picture, vec![Step::NotShown]), (Who::Drawing, vec![Step::Off]), (Who::Hdmi, vec![Step::Off])];
        // asked by a setting changed during the recording: the paused output asks for its frame
        let paused = |x| p.give_way_or_go(Who::Hdmi, Step::Off, &ladders, by(&PAUSED, x));
        assert_eq!(paused(10), Some(vec![(Who::Picture, Step::NotShown)]), "the pictures first");
        assert_eq!(paused(15), Some(vec![(Who::Drawing, Step::Off)]), "then the drawing");
        assert_eq!(paused(30), Some(vec![(Who::Hdmi, Step::Off)]), "then it goes dark");
        assert_eq!(paused(50), Some(vec![(Who::Picture, Step::NotShown), (Who::Drawing, Step::Off), (Who::Hdmi, Step::Off)]));
        assert_eq!(paused(70), None);
        // asked by the recording starting: its off is its rung among the holders below the recording
        let rec = |x| p.give_way(Who::Recording, &ladders, by(&PAUSED, x));
        assert_eq!(rec(10), Some(vec![(Who::Picture, Step::NotShown)]));
        assert_eq!(rec(15), Some(vec![(Who::Drawing, Step::Off)]));
        assert_eq!(rec(30), Some(vec![(Who::Hdmi, Step::Off)]));
        // above the drawing and the pictures in the queue: they are never taken for it
        let q = Precedence::parse("recording > player > usb > photo > drawing > picture > hdmi").unwrap();
        assert_eq!(q.give_way_or_go(Who::Hdmi, Step::Off, &ladders, by(&PAUSED, 10)), Some(vec![(Who::Hdmi, Step::Off)]));
    }

    #[test]
    fn a_floor_at_paused_refuses_the_asker_instead() {
        let p = Precedence::parse("recording > player > usb > hdmi:paused > photo > drawing > picture").unwrap();
        let ladders = vec![(Who::Picture, vec![Step::NotShown]), (Who::Drawing, vec![Step::Off]), (Who::Hdmi, vec![Step::Off])];
        assert_eq!(p.give_way_or_go(Who::Hdmi, Step::Off, &ladders, by(&PAUSED, 40)), None);
        assert_eq!(p.give_way(Who::Recording, &ladders, by(&PAUSED, 40)), None);
        // what the holders below free is still taken
        assert_eq!(p.give_way_or_go(Who::Hdmi, Step::Off, &ladders, by(&PAUSED, 15)), Some(vec![(Who::Drawing, Step::Off)]));
    }
}
