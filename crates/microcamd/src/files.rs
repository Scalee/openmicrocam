//! The card's file list (`microcam_core::files`) kept in memory: read off the card whenever it comes, goes, or is
//! formatted (`card` tells), a file of ours added once it is whole (a photo written, a recording closed) — the card's
//! space read again then. A `files`
//! request is answered from it on the asking thread, the card untouched, so it holds no later request; the
//! `files {version, total}` event goes to every follower at each change, the list stored before it is told.

use std::path::Path;
use std::sync::{Arc, Mutex};

use microcam_core::files::{parse, FileEntry, List};
use microcam_core::protocol::{Card, Event, FilesInfo, FilesWindow};

use crate::card;
use crate::followers::Followers;

pub struct Files {
    list: Mutex<List>,
    followers: Arc<Followers>,
}

impl Files {
    pub fn new(followers: Arc<Followers>) -> Arc<Self> {
        Arc::new(Self { list: Mutex::default(), followers })
    }

    pub fn info(&self) -> FilesInfo {
        let l = self.list.lock().unwrap();
        FilesInfo { version: l.version(), total: l.total() }
    }

    pub fn window(&self, from: usize, count: usize) -> FilesWindow {
        let l = self.list.lock().unwrap();
        FilesWindow { from, total: l.total(), version: l.version(), list: l.window(from, count).to_vec() }
    }

    /// The number of a new photo or video.
    pub fn next(&self) -> u32 {
        self.list.lock().unwrap().new_number()
    }

    /// The card is now `at`: its folders read when it is here, the list empty otherwise.
    pub fn card(&self, at: Card) {
        let entries = if at == Card::Camera { read() } else { Vec::new() };
        let changed = self.list.lock().unwrap().replace(entries);
        if changed {
            self.tell();
        }
    }

    /// A file of ours, whole on the card now, at `path`.
    pub fn add(&self, path: &str) {
        let p = Path::new(path);
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else { return };
        let Some((kind, number)) = parse(name) else { return };
        let bytes = std::fs::metadata(p).map_or(0, |m| m.len());
        let changed = self.list.lock().unwrap().add(FileEntry { name: name.into(), path: path.into(), kind, number, bytes });
        if changed {
            self.tell();
        }
        card::space_changed();
    }

    fn tell(&self) {
        self.followers.tell_with(|| {
            let l = self.list.lock().unwrap();
            vec![Event::Files { version: l.version(), total: l.total() }]
        });
    }
}

/// The photos and videos in the card's folders.
fn read() -> Vec<FileEntry> {
    let mut out = Vec::new();
    for dir in ["PHOTO", "VIDEO"] {
        let Ok(rd) = std::fs::read_dir(format!("{}/{dir}", card::CARD)) else { continue };
        for e in rd.flatten() {
            let Some(name) = e.file_name().to_str().map(String::from) else { continue };
            let Some((kind, number)) = parse(&name) else { continue };
            let Some(path) = e.path().to_str().map(String::from) else { continue };
            let bytes = e.metadata().map_or(0, |m| m.len());
            out.push(FileEntry { name, path, kind, number, bytes });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::followers::{events, Client};

    fn files() -> (Arc<Files>, Arc<Followers>, std::sync::mpsc::Receiver<String>) {
        let followers = Arc::new(Followers::default());
        let (c, rx) = Client::test(1);
        followers.watch(c, String::new);
        (Files::new(followers.clone()), followers, rx)
    }

    /// Files of ours join the list newest first, each change told with the version it made; a file already listed or
    /// not of the camera's changes nothing; the card gone empties it.
    #[test]
    fn added_windowed_and_replaced() {
        let (f, _, rx) = files();
        f.add("/mnt/PHOTO/P00001.jpg");
        f.add("/mnt/VIDEO/V00002.mp4");
        f.add("/mnt/PHOTO/P00001.jpg");
        f.add("/mnt/PHOTO/notes.txt");
        assert_eq!(events(&rx), [Event::Files { version: 1, total: 1 }, Event::Files { version: 2, total: 2 }]);
        assert_eq!(f.next(), 3);
        let w = f.window(0, 1);
        assert_eq!((w.version, w.total, w.list.iter().map(|e| e.name.as_str()).collect::<Vec<_>>()), (2, 2, vec!["V00002.mp4"]));
        assert_eq!(f.window(1, 5).list[0].path, "/mnt/PHOTO/P00001.jpg");
        assert!(f.window(5, usize::MAX).list.is_empty());
        f.card(Card::None);
        assert_eq!(events(&rx), [Event::Files { version: 3, total: 0 }]);
        f.card(Card::Host);
        assert_eq!(events(&rx), [], "empty already");
    }

    /// The list changing while a follower subscribes: its snapshot comes first, and no event after it carries an older
    /// version than the snapshot's.
    #[test]
    fn the_snapshot_comes_before_every_event_after_it() {
        let (f, followers, _) = files();
        let writer = {
            let f = f.clone();
            std::thread::spawn(move || {
                for n in 1..=2000 {
                    f.add(&format!("/mnt/PHOTO/P{n:05}.jpg"));
                }
            })
        };
        let (c, rx) = Client::test(2);
        followers.watch(c, || format!("snapshot {}", f.info().version));
        writer.join().unwrap();
        let lines: Vec<String> = rx.try_iter().collect();
        let mut last: u64 = lines[0].strip_prefix("snapshot ").expect("the snapshot first").parse().unwrap();
        for l in &lines[1..] {
            let Event::Files { version: v, .. } = serde_json::from_str(l).unwrap() else { panic!("{l}") };
            assert!(v >= last, "an event older than what came before it: {v} after {last}");
            last = v;
        }
        assert_eq!(last, 2000, "the last change told");
    }
}
