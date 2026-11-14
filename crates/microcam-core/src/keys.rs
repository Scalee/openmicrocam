//! The remote's keys: each key's name (what the remote prints on it) by the kernel's `EV_KEY` code, the kernel's input
//! events as they are on this 32-bit ARM, and the rule of whom a key goes to: the scene's owner it was pressed under.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A key of the remote, by what the remote prints on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Menu,
    ZoomIn,
    ZoomOut,
    Mirror,
    Flip,
    Ae,
    Awb,
    Cross,
    Picture,
    Freeze,
    Play,
    Photo,
    Record,
    Gallery,
}

/// The 18 keys of the remote by their `EV_KEY` code (the kernel's keymap for it).
pub const KEYS: [(u16, Key); 18] = [
    (103, Key::Up),
    (108, Key::Down),
    (105, Key::Left),
    (106, Key::Right),
    (102, Key::Menu),
    (116, Key::ZoomIn),
    (11, Key::ZoomOut),
    (2, Key::Mirror),
    (4, Key::Flip),
    (3, Key::Ae),
    (6, Key::Awb),
    (5, Key::Cross),
    (7, Key::Picture),
    (8, Key::Freeze),
    (9, Key::Play),
    (10, Key::Photo),
    (46, Key::Record),
    (59, Key::Gallery),
];

impl Key {
    /// The key of an `EV_KEY` code.
    pub fn of(code: u16) -> Option<Key> {
        KEYS.iter().find(|k| k.0 == code).map(|k| k.1)
    }

    pub fn code(self) -> u16 {
        KEYS.iter().find(|k| k.1 == self).map(|k| k.0).expect("every key is in KEYS")
    }
}

/// What a `key` request does to a key: the kernel's repeats come of a press held.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Push {
    Press,
    Release,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    Press,
    /// the kernel's autorepeat while the key is held
    Repeat,
    Release,
}

impl Phase {
    /// From an `EV_KEY` event's value: 1 press, 2 repeat, 0 release.
    pub fn of(value: i32) -> Option<Phase> {
        match value {
            1 => Some(Phase::Press),
            2 => Some(Phase::Repeat),
            0 => Some(Phase::Release),
            _ => None,
        }
    }
}

pub const EV_SYN: u16 = 0;
pub const EV_KEY: u16 = 1;
/// `struct input_event` on 32-bit ARM: a `timeval` of two 32-bit words, then type, code and value.
pub const EVENT: usize = 16;

/// An input event's type, code and value.
pub fn parse(e: &[u8; EVENT]) -> (u16, u16, i32) {
    (u16::from_le_bytes([e[8], e[9]]), u16::from_le_bytes([e[10], e[11]]), i32::from_le_bytes([e[12], e[13], e[14], e[15]]))
}

/// An input event to write into the device (the kernel stamps its time).
pub fn event(kind: u16, code: u16, value: i32) -> [u8; EVENT] {
    let mut e = [0u8; EVENT];
    e[8..10].copy_from_slice(&kind.to_le_bytes());
    e[10..12].copy_from_slice(&code.to_le_bytes());
    e[12..16].copy_from_slice(&value.to_le_bytes());
    e
}

/// Who each held key goes to: the owner it was pressed under, as long as that owner stays.
#[derive(Default)]
pub struct Router {
    /// a held key and the owner it was pressed under (None: there was none)
    held: BTreeMap<u16, Option<u64>>,
}

impl Router {
    /// Whether this key event goes to `owner` (the scene's owner now, by its generation; None: no owner).
    pub fn route(&mut self, code: u16, phase: Phase, owner: Option<u64>) -> bool {
        let under = match phase {
            Phase::Press => {
                self.held.insert(code, owner);
                owner
            }
            Phase::Repeat => self.held.get(&code).copied().flatten(),
            Phase::Release => self.held.remove(&code).flatten(),
        };
        owner.is_some() && under == owner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_has_one_name_and_one_code() {
        for (c, k) in KEYS {
            assert_eq!((k.code(), Key::of(c)), (c, Some(k)));
        }
        let mut codes: Vec<u16> = KEYS.iter().map(|k| k.0).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), KEYS.len());
        assert_eq!((Key::of(103), Key::of(11), Key::of(46), Key::of(59), Key::of(1)), (Some(Key::Up), Some(Key::ZoomOut), Some(Key::Record), Some(Key::Gallery), None));
        assert_eq!(serde_json::to_string(&Key::ZoomIn).unwrap(), r#""zoom-in""#);
        assert!(serde_json::from_str::<Key>(r#""ok""#).is_err());
    }

    #[test]
    fn a_key_goes_to_the_owner_it_was_pressed_under() {
        let mut r = Router::default();
        assert!(r.route(103, Phase::Press, Some(1)));
        assert!(r.route(103, Phase::Repeat, Some(1)));
        assert!(!r.route(103, Phase::Repeat, Some(2)), "the owner changed: its repeats go nowhere");
        assert!(!r.route(103, Phase::Release, Some(2)), "nor its release");
        assert!(r.route(103, Phase::Press, Some(2)), "the next press goes to the new owner");
        assert!(r.route(103, Phase::Release, Some(2)));
        assert!(!r.route(108, Phase::Press, None), "no owner, nowhere");
        assert!(!r.route(108, Phase::Repeat, Some(3)), "pressed before the owner came: nowhere");
        assert!(!r.route(108, Phase::Release, Some(3)));
        assert!(!r.route(105, Phase::Release, Some(3)), "a release never pressed");
    }

    #[test]
    fn input_events_are_16_bytes_little_endian() {
        let e = event(EV_KEY, 103, 2);
        assert_eq!(parse(&e), (EV_KEY, 103, 2));
        assert_eq!(&e[8..], &[1, 0, 103, 0, 2, 0, 0, 0]);
        assert_eq!((Phase::of(1), Phase::of(2), Phase::of(0), Phase::of(3)), (Some(Phase::Press), Some(Phase::Repeat), Some(Phase::Release), None));
    }
}
