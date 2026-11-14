//! The card's photos and videos as the menu's gallery sees them: one list, newest first by number (the camera has no
//! clock, so the number is the order they were made in), a version raised at every change of the set, and the next
//! number for a new file of either kind.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileKind {
    Photo,
    Video,
}

/// A file of the list: its name, its path on the camera (what `play` and a `file` resource take), kind, number and bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub kind: FileKind,
    pub number: u32,
    pub bytes: u64,
}

/// A card file's kind and number from its name: `P#####.jpg` in `PHOTO/`, `V#####.mp4` in `VIDEO/` — the number as
/// [`name_number`] writes it, five digits at least — a second take of a number as `V#####_2.mp4`; None for any other
/// file, whatever its name.
pub fn parse(name: &str) -> Option<(FileKind, u32)> {
    let b = name.as_bytes();
    let (kind, ext): (_, &[u8]) = match b.first()? {
        b'P' => (FileKind::Photo, b".jpg"),
        b'V' => (FileKind::Video, b".mp4"),
        _ => return None,
    };
    let rest = &b[1..];
    let n = rest.len().checked_sub(ext.len())?;
    if !rest[n..].eq_ignore_ascii_case(ext) {
        return None;
    }
    let all_digits = |d: &[u8]| !d.is_empty() && d.iter().all(u8::is_ascii_digit);
    let mut parts = rest[..n].splitn(2, |&c| c == b'_');
    let digits = parts.next()?;
    if !all_digits(digits) || parts.next().is_some_and(|take| !all_digits(take)) {
        return None;
    }
    let number: u32 = std::str::from_utf8(digits).ok()?.parse().ok()?;
    (name_number(number).as_bytes() == digits).then_some((kind, number))
}

/// A file's number as its name carries it.
pub fn name_number(n: u32) -> String {
    format!("{n:05}")
}

/// The list and its version.
#[derive(Default)]
pub struct List {
    entries: Vec<FileEntry>,
    version: u64,
    /// the highest number given out for a file not in the list yet (a photo being taken, a recording running)
    given: u32,
}

impl List {
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn total(&self) -> usize {
        self.entries.len()
    }

    /// The list as read off the card (the card came, went, was formatted); the version raised when it differs.
    pub fn replace(&mut self, mut entries: Vec<FileEntry>) -> bool {
        sort(&mut entries);
        self.given = 0;
        if entries == self.entries {
            return false;
        }
        self.entries = entries;
        self.version += 1;
        true
    }

    /// A file of ours added (its photo saved, its recording closed); the version raised when it is new or changed.
    pub fn add(&mut self, e: FileEntry) -> bool {
        if self.entries.contains(&e) {
            return false;
        }
        self.entries.retain(|x| x.name != e.name);
        self.entries.push(e);
        sort(&mut self.entries);
        self.version += 1;
        true
    }

    /// `count` files from place `from` (newest first).
    pub fn window(&self, from: usize, count: usize) -> &[FileEntry] {
        let from = from.min(self.entries.len());
        &self.entries[from..from.saturating_add(count).min(self.entries.len())]
    }

    /// The number of a new photo or video: the next after the highest of both folders and of those given out.
    pub fn new_number(&mut self) -> u32 {
        let top = self.entries.iter().map(|e| e.number).max().unwrap_or(0).max(self.given);
        // past the last number the last again: its file is then refused, never written over (made only new)
        self.given = top.saturating_add(1);
        self.given
    }
}

/// Newest first: the higher number, at one number the video first, then by name.
fn sort(entries: &mut [FileEntry]) {
    entries.sort_by(|a, b| b.number.cmp(&a.number).then((b.kind == FileKind::Video).cmp(&(a.kind == FileKind::Video))).then(b.name.cmp(&a.name)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> FileEntry {
        let (kind, number) = parse(name).unwrap();
        FileEntry { name: name.into(), path: format!("/card/{name}"), kind, number, bytes: 1 }
    }

    #[test]
    fn names_give_kind_and_number() {
        assert_eq!(parse("P00012.jpg"), Some((FileKind::Photo, 12)));
        assert_eq!(parse("V00003.MP4"), Some((FileKind::Video, 3)));
        assert_eq!(parse("V00003_2.mp4"), Some((FileKind::Video, 3)));
        for bad in ["P0012.jpg", "P00012.mp4", "X00001.jpg", "V00003_.mp4", "V00003_a.mp4", "P00012.jpgx", "", "P"] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn any_name_parses_without_panic() {
        for odd in ["P😀x", "P😀.jpg", "Pé00001.jpg", "V00001_😀.mp4", "😀00001.jpg", "P.jpg", "P_.jpg", "V_1.mp4", "P4294967296.jpg"] {
            assert_eq!(parse(odd), None, "{odd}");
        }
    }

    #[test]
    fn numbers_past_five_digits_read_back_as_written() {
        for n in [0, 99_999, 100_000, 1_234_567, u32::MAX] {
            assert_eq!(parse(&format!("P{}.jpg", name_number(n))), Some((FileKind::Photo, n)));
            assert_eq!(parse(&format!("V{}_2.mp4", name_number(n))), Some((FileKind::Video, n)));
        }
        // a number written another way is not one of ours
        assert_eq!(parse("P012345.jpg"), None);
        let mut l = List::default();
        l.replace(vec![entry("P99999.jpg"), entry("P100000.jpg")]);
        assert_eq!(l.new_number(), 100_001, "after a reboot the six-digit file still counts");
        l.replace(vec![entry(&format!("P{}.jpg", name_number(u32::MAX)))]);
        assert_eq!(l.new_number(), u32::MAX, "no overflow: the last number again, refused when made");
    }

    #[test]
    fn one_numbering_newest_first_with_a_window_and_a_version() {
        let mut l = List::default();
        assert!(l.replace(vec![entry("P00001.jpg"), entry("V00002.mp4"), entry("P00002.jpg"), entry("P00007.jpg"), entry("V00003.mp4")]));
        let names: Vec<&str> = l.window(0, 10).iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["P00007.jpg", "V00003.mp4", "V00002.mp4", "P00002.jpg", "P00001.jpg"], "a shared number: the video first");
        assert_eq!(l.version(), 1);
        assert!(!l.replace(l.window(0, 10).to_vec()), "the same set: no new version");
        assert_eq!(l.version(), 1);
        assert_eq!(l.window(1, 2).len(), 2);
        assert_eq!(l.window(4, 5).len(), 1);
        assert!(l.window(9, 5).is_empty());
        assert_eq!(l.window(1, usize::MAX).len(), 4, "a count to the end: no overflow");
        assert!(l.window(usize::MAX, usize::MAX).is_empty());
        // a recording and a photo taken during it: two numbers, both above every file
        assert_eq!((l.new_number(), l.new_number()), (8, 9));
        assert!(l.add(entry("P00009.jpg")));
        assert!(!l.add(entry("P00009.jpg")), "added twice: one entry, one version");
        assert_eq!((l.total(), l.version()), (6, 2));
        assert_eq!(l.new_number(), 10, "the recording's 8 is still given out");
        // the card read again (another card): the numbers start from its files
        l.replace(vec![entry("P00003.jpg")]);
        assert_eq!((l.new_number(), l.version()), (4, 3));
    }
}
