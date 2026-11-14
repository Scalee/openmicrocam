//! A reader's pace through the frames of a source that may run faster: each frame's capture time against the reader's
//! own next one decides whether it takes that frame. Whatever the ratio of the two rates, and as the source's rate
//! changes, the reader takes its rate of them, spread evenly; at the source's rate or above it takes every frame.

/// The frames a second a reader takes, and the capture time (µs) its next frame is due at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pace {
    interval: u64,
    due: u64,
}

impl Pace {
    pub fn new(fps: u32) -> Self {
        Self { interval: 1_000_000 / fps.max(1) as u64, due: 0 }
    }

    /// Its rate changed: the next frame due a new interval after the last one taken.
    pub fn set(&mut self, fps: u32) {
        let interval = 1_000_000 / fps.max(1) as u64;
        if interval != self.interval {
            self.due = self.due.saturating_sub(self.interval) + interval;
            self.interval = interval;
        }
    }

    /// Whether the frame captured at `pts` (µs) is the reader's: one within half an interval of its due time, or past
    /// it. A frame taken moves the due time an interval on — from the frame itself when the source paused or jumped
    /// past a whole interval, so a gap brings no burst.
    pub fn take(&mut self, pts: u64) -> bool {
        if self.due != 0 && pts + self.interval / 2 < self.due {
            return false;
        }
        self.due = if self.due == 0 || pts > self.due + self.interval { pts + self.interval } else { self.due + self.interval };
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frames of `n` taken from a source at `src` fps, by a reader at `fps`.
    fn taken(src: u32, fps: u32, n: u64) -> Vec<u64> {
        let mut p = Pace::new(fps);
        (0..n).filter(|i| p.take(1_000 + i * 1_000_000 / src as u64)).collect()
    }

    #[test]
    fn a_slower_reader_takes_its_rate_evenly_whatever_the_ratio() {
        // 25 → 7: 7 a second, steps of 3 and 4 frames after the first
        let t = taken(25, 7, 250);
        assert!((69..=71).contains(&t.len()), "{}", t.len());
        assert!(t[1..].windows(2).all(|w| (3..=4).contains(&(w[1] - w[0]))), "{t:?}");
        // 45 → 30, 30 → 7, 25 → 25
        assert!((299..=301).contains(&taken(45, 30, 450).len()));
        assert!((69..=71).contains(&taken(30, 7, 300).len()));
        assert_eq!(taken(25, 25, 250).len(), 250);
    }

    #[test]
    fn a_reader_at_or_above_the_source_takes_every_frame() {
        assert_eq!(taken(25, 30, 100).len(), 100);
        assert_eq!(taken(30, 30, 100).len(), 100);
    }

    #[test]
    fn the_source_s_rate_changing_needs_nothing_and_a_gap_brings_no_burst() {
        let mut p = Pace::new(7);
        let mut pts = 0;
        let mut n = 0;
        // a second at 45, a second at 25: 7 in each
        for (src, frames) in [(45u64, 45), (25, 25)] {
            let before = n;
            for _ in 0..frames {
                pts += 1_000_000 / src;
                n += p.take(pts) as u32;
            }
            assert!((6..=8).contains(&(n - before)), "{src}: {}", n - before);
        }
        // a 2-s gap: the next frame taken, then the pace again
        pts += 2_000_000;
        assert!(p.take(pts));
        assert!(!p.take(pts + 40_000));
    }

    #[test]
    fn a_new_rate_holds_from_the_last_frame_taken() {
        let mut p = Pace::new(25);
        assert!(p.take(1_000));
        p.set(7);
        // within half the new interval of 1 000 + 142 857
        assert!(!p.take(41_000));
        assert!(p.take(81_000));
        assert!(!p.take(121_000) && !p.take(161_000) && !p.take(201_000));
    }
}
