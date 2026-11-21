//! What takes time and runs now — the photo being taken, the recording starting · running · stopping, the card
//! formatting, the controls being built — one of each kind at a time. A request that begins one is admitted here: the
//! state turns busy before it is answered. Its outcome goes to every follower as the state is freed; admitting and
//! ending both happen under the followers' lock, so the outcome is queued before the next of its kind can be admitted
//! (lock order: `followers`).

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use microcam_core::controls::{Code, Refusal};
use microcam_core::protocol::{Action, Card, Event, RecordRun, RecordState, Running as Info};

use crate::card;
use crate::files::Files;
use crate::followers::Followers;

/// Whoever follows the recording's acceptance and end ([`Running::on_record`]): the events of what it changed.
pub type OnRecord = Box<dyn Fn(bool) -> Vec<Event> + Send + Sync>;

pub struct Running {
    st: Mutex<State>,
    followers: Arc<Followers>,
    /// told when a recording is accepted (true) and when it has ended (false), under the followers' lock: the events
    /// its change makes, told before what follows (an ended recording's outcome)
    on_record: Mutex<Option<OnRecord>>,
    /// the card's list: a photo written and a recording closed join it before their outcome is told
    files: Arc<Files>,
}

#[derive(Default)]
struct State {
    photo: Option<String>,
    record: Option<Record>,
    format: bool,
    building: BTreeSet<String>,
}

struct Record {
    file: String,
    state: RecordState,
    /// when it began to run
    since: Option<Instant>,
    /// stopping because `record-stop` asked (not because the card went or its writer failed)
    asked: bool,
    /// its writer failed before the pipeline stopped it (the card full, gone, read-only): why, told already
    failed: Option<String>,
}

impl Record {
    /// Its outcome once the pipeline has stopped it and its writer has closed the file (`closed`).
    fn outcome(&self, closed: &Result<String, String>) -> Event {
        match (self.asked, closed) {
            (true, Ok(file)) => Event::Recorded { file: file.clone() },
            (true, Err(e)) => Event::Failed { what: Action::RecordStop, code: Code::Refused, error: e.clone(), holders: Vec::new() },
            (false, Ok(file)) => Event::Failed { what: Action::Record, code: Code::NoCard, error: format!("the card went: {file} stopped"), holders: Vec::new() },
            (false, Err(e)) => Event::Failed { what: Action::Record, code: Code::NoCard, error: format!("the card went: {e}"), holders: Vec::new() },
        }
    }
}

impl State {
    fn info(&self) -> Info {
        Info {
            photo: self.photo.clone(),
            record: self.record.as_ref().map(|r| RecordRun { file: r.file.clone(), state: r.state, ms: r.since.map_or(0, |t| t.elapsed().as_millis() as u64) }),
            format: self.format,
            building: self.building.iter().cloned().collect(),
        }
    }

    /// What runs, without the recording's time: whether anything changed.
    fn key(&self) -> Info {
        let mut i = self.info();
        if let Some(r) = i.record.as_mut() {
            r.ms = 0;
        }
        i
    }

    /// Why the card cannot be written now.
    fn card(&self) -> Result<(), Refusal> {
        if self.format {
            return Err(Refusal::new(Code::Formatting, "the card is being formatted"));
        }
        match card::now() {
            Card::Camera => Ok(()),
            Card::Host => Err(Refusal::new(Code::CardShared, "the SD card is with the USB host")),
            Card::None => Err(Refusal::new(Code::NoCard, "no SD card")),
            Card::Unreadable => Err(Refusal::new(Code::CardUnreadable, "the camera cannot read the SD card")),
        }
    }
}

fn playing(playing: bool) -> Result<(), Refusal> {
    if playing {
        Err(Refusal::new(Code::Playing, "a file is playing"))
    } else {
        Ok(())
    }
}

impl Running {
    pub fn new(followers: Arc<Followers>, files: Arc<Files>) -> Arc<Self> {
        Arc::new(Self { st: Mutex::default(), followers, files, on_record: Mutex::default() })
    }

    /// Who hears that a recording was accepted or has ended (the controls: the record ones are its meanwhile).
    pub fn on_record(&self, f: OnRecord) {
        *self.on_record.lock().unwrap() = Some(f);
    }

    /// The events of the recording's acceptance (`on`) or end, from [`Self::on_record`]; called under the followers'
    /// lock, this state's let go.
    fn record_told(&self, on: bool) -> Vec<Event> {
        self.on_record.lock().unwrap().as_ref().map_or_else(Vec::new, |f| f(on))
    }

    /// What runs now, as the snapshot tells it.
    pub fn info(&self) -> Info {
        self.st.lock().unwrap().info()
    }

    /// `f` admits a request into the state or refuses it; admitted, every follower is told what runs now.
    fn admit(&self, f: impl FnOnce(&mut State) -> Result<(), Refusal>) -> Result<(), Refusal> {
        let mut r = Ok(());
        self.followers.tell_with(|| {
            let mut st = self.st.lock().unwrap();
            r = f(&mut st);
            if r.is_ok() {
                vec![Event::Running { running: st.info() }]
            } else {
                Vec::new()
            }
        });
        r
    }

    /// An ending: the state as `f` frees it, the outcome `f` gives told first, what runs now after it when it changed.
    /// A recording it ends is told ended before both, so whoever hears the outcome finds its controls free.
    fn end(&self, f: impl FnOnce(&mut State) -> Option<Event>) {
        self.followers.tell_with(|| {
            let mut st = self.st.lock().unwrap();
            let before = st.key();
            let had = st.record.is_some();
            let outcome = f(&mut st);
            let ended = had && st.record.is_none();
            let changed = (st.key() != before).then(|| Event::Running { running: st.info() });
            drop(st);
            let ended = if ended { self.record_told(false) } else { Vec::new() };
            ended.into_iter().chain(outcome).chain(changed).collect()
        });
    }

    /// A photo into `file`: refused without a card to write, while a file plays or a photo is being taken.
    pub fn photo(&self, file: &str, is_playing: bool) -> Result<(), Refusal> {
        self.admit(|st| {
            st.card()?;
            playing(is_playing)?;
            if st.photo.is_some() {
                return Err(Refusal::new(Code::Busy, "a photo is being taken"));
            }
            st.photo = Some(file.to_string());
            Ok(())
        })
    }

    /// The photo written into its file, or why not.
    pub fn photo_done(&self, r: Result<(), Refusal>) {
        // the file read and this lock let go before the list tells the followers (their lock comes first)
        let file = self.st.lock().unwrap().photo.clone();
        if let (Ok(()), Some(file)) = (&r, file) {
            self.files.add(&file);
        }
        self.end(|st| {
            let file = st.photo.take()?;
            Some(match r {
                Ok(()) => Event::PhotoSaved { file },
                Err(r) => Event::failed(Action::Photo, r),
            })
        });
    }

    /// A recording into `file`: refused without a card to write, while a file plays or one records.
    pub fn record(&self, file: &str, is_playing: bool) -> Result<(), Refusal> {
        self.admit(|st| {
            st.card()?;
            playing(is_playing)?;
            match st.record.as_ref().map(|r| r.state) {
                Some(RecordState::Running) => return Err(Refusal::new(Code::Recording, "a recording is running")),
                Some(_) => return Err(Refusal::new(Code::Busy, "a recording is starting or stopping")),
                None => {}
            }
            st.record = Some(Record { file: file.to_string(), state: RecordState::Starting, since: None, asked: false, failed: None });
            Ok(())
        })?;
        // its controls are its before it is answered
        self.followers.tell_with(|| self.record_told(true));
        Ok(())
    }

    /// The pipeline built the recording asked for, or why not.
    pub fn record_built(&self, r: Result<(), Refusal>) {
        self.end(|st| match r {
            Ok(()) => {
                if let Some(rec) = st.record.as_mut() {
                    rec.state = RecordState::Running;
                    rec.since = Some(Instant::now());
                }
                None
            }
            Err(r) => st.record.take().map(|_| Event::failed(Action::Record, r)),
        });
    }

    /// `record-stop`: refused when nothing records, or while it starts or stops.
    pub fn record_stop(&self) -> Result<(), Refusal> {
        self.admit(|st| match st.record.as_mut() {
            None => Err(Refusal::new(Code::Unavailable, "nothing is recording")),
            Some(r) if r.state != RecordState::Running => Err(Refusal::new(Code::Busy, "the recording is starting or stopping")),
            Some(r) => {
                r.state = RecordState::Stopping;
                r.asked = true;
                Ok(())
            }
        })
    }

    /// The pipeline ended the recording, as asked or by itself (the card gone, its writer failed): it stays stopping
    /// until its writer has closed the file ([`Self::record_closed`]); its writer failed already, it ends now (told
    /// when it failed).
    pub fn record_stopped(&self) {
        self.end(|st| {
            let r = st.record.as_mut()?;
            if r.failed.is_some() {
                st.record = None;
                return None;
            }
            r.state = RecordState::Stopping;
            None
        });
    }

    /// The recording's writer ended: the file closed whole on the card (`recorded {file}`), or why not (`failed`).
    /// Failed before the pipeline stopped the recording: told now, and true — the recording is to be stopped, as when
    /// the card goes. Closed whole before the stop (the recording restarted into a new file): that file is listed,
    /// the outcome is the last writer's.
    pub fn record_closed(&self, closed: Result<String, String>) -> bool {
        if let Ok(file) = &closed {
            self.files.add(file);
        }
        let mut stop = false;
        self.end(|st| {
            let r = st.record.as_mut()?;
            if r.state == RecordState::Stopping {
                let e = r.outcome(&closed);
                st.record = None;
                return Some(e);
            }
            let e = closed.err()?;
            stop = r.failed.is_none();
            r.failed = Some(e.clone());
            Some(Event::Failed { what: Action::Record, code: Code::Refused, error: e, holders: Vec::new() })
        });
        stop
    }

    /// The stop asked could not be built: the recording goes on.
    pub fn record_stop_failed(&self, e: String) {
        self.end(|st| {
            let r = st.record.as_mut()?;
            r.state = RecordState::Running;
            r.asked = false;
            Some(Event::Failed { what: Action::RecordStop, code: Code::Refused, error: e, holders: Vec::new() })
        });
    }

    /// `format`: refused without the card here, while it is written or read (a recording, a photo, a file playing).
    pub fn format(&self, is_playing: bool) -> Result<(), Refusal> {
        self.admit(|st| {
            if st.format {
                return Err(Refusal::new(Code::Busy, "the card is being formatted"));
            }
            // a card the camera cannot read is formatted all the same
            if card::now() != Card::Unreadable {
                st.card()?;
            }
            if st.record.is_some() {
                return Err(Refusal::new(Code::Recording, "a recording is running"));
            }
            playing(is_playing)?;
            if st.photo.is_some() {
                return Err(Refusal::new(Code::Busy, "a photo is being taken"));
            }
            st.format = true;
            Ok(())
        })
    }

    pub fn format_done(&self, r: Result<(), String>) {
        let outcome = match r {
            Ok(()) => Event::Formatted,
            Err(e) => Event::Failed { what: Action::Format, code: Code::Refused, error: e, holders: Vec::new() },
        };
        self.end(|st| {
            st.format = false;
            Some(outcome)
        });
    }

    /// Why the card cannot be handed to the USB host or taken back now: it is being formatted.
    pub fn card_free(&self) -> Result<(), Refusal> {
        if self.st.lock().unwrap().format {
            return Err(Refusal::new(Code::Formatting, "the card is being formatted"));
        }
        Ok(())
    }

    /// Why a file cannot be played now: the card being formatted, a recording.
    pub fn play_free(&self) -> Result<(), Refusal> {
        self.card_free()?;
        if self.st.lock().unwrap().record.is_some() {
            return Err(Refusal::new(Code::Recording, "a recording is running"));
        }
        Ok(())
    }

    /// Controls `names` to be built: refused while any of them is.
    pub fn build(&self, names: &[String]) -> Result<(), Refusal> {
        self.admit(|st| {
            if let Some(n) = names.iter().find(|n| st.building.contains(*n)) {
                return Err(Refusal::new(Code::Busy, format!("{n} is being built")));
            }
            st.building.extend(names.iter().cloned());
            Ok(())
        })
    }

    /// The build of `names` ended: built (their `control` events went already), or why not, said as `what`.
    pub fn built(&self, what: Action, names: &[String], r: Result<(), Refusal>) {
        self.end(|st| {
            for n in names {
                st.building.remove(n);
            }
            r.err().map(|r| Event::failed(what, r))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::followers::{events, Client, Followers};
    use std::sync::mpsc::Receiver;
    use std::time::Duration;

    struct Bench {
        running: Arc<Running>,
        files: Arc<Files>,
        followers: Arc<Followers>,
        /// a watcher's lines
        rx: Receiver<String>,
        /// what `on_record` heard
        record_told: Arc<Mutex<Vec<bool>>>,
    }

    fn bench() -> Bench {
        card::here();
        let followers = Arc::new(Followers::default());
        let files = Files::new(followers.clone());
        let running = Running::new(followers.clone(), files.clone());
        let record_told = Arc::new(Mutex::new(Vec::new()));
        let t = record_told.clone();
        running.on_record(Box::new(move |on| {
            t.lock().unwrap().push(on);
            vec![codec(!on)]
        }));
        let (c, rx) = Client::test(1);
        followers.watch(c, String::new);
        Bench { running, files, followers, rx, record_told }
    }

    fn run(info: Info) -> Event {
        Event::Running { running: info }
    }

    /// What the bench's controls tell when a recording is accepted (not free) or has ended (free).
    fn codec(free: bool) -> Event {
        let v = microcam_core::controls::Value::Text("h264".into());
        let control = microcam_core::controls::Control { name: "record-codec".into(), kind: microcam_core::controls::Kind::Choice, min: None, max: None, step: None, values: Vec::new(), default: v.clone(), value: v, kept: true, available: free, why: None };
        Event::Control { control }
    }

    fn rec(file: &str, state: RecordState) -> Info {
        Info { record: Some(RecordRun { file: file.into(), state, ms: 0 }), ..Info::default() }
    }

    /// The events told since the last look, a recording's time zeroed.
    fn told(b: &Bench) -> Vec<Event> {
        let mut e = events(&b.rx);
        for ev in &mut e {
            if let Event::Running { running: Info { record: Some(r), .. } } = ev {
                r.ms = 0;
            }
        }
        e
    }

    const P1: &str = "/mnt/PHOTO/P00001.jpg";
    const V1: &str = "/mnt/VIDEO/V00001.mp4";
    const V1_2: &str = "/mnt/VIDEO/V00001_2.mp4";

    /// A photo is admitted once: busy until it ends; while a file plays it is refused. Written, it joins the list before
    /// `photo-saved`, then what runs is told; failed, `failed` and no file.
    #[test]
    fn a_photo_is_admitted_once_and_ends_with_its_outcome() {
        let b = bench();
        assert_eq!(b.running.photo(P1, true).unwrap_err().code, Code::Playing);
        b.running.photo(P1, false).unwrap();
        assert_eq!(b.running.photo("/mnt/PHOTO/P00002.jpg", false).unwrap_err().code, Code::Busy);
        assert_eq!(b.running.format(false).unwrap_err().code, Code::Busy);
        b.running.photo_done(Ok(()));
        let photo = Info { photo: Some(P1.into()), ..Info::default() };
        assert_eq!(told(&b), [run(photo), Event::Files { version: 1, total: 1 }, Event::PhotoSaved { file: P1.into() }, run(Info::default())]);
        b.running.photo("/mnt/PHOTO/P00002.jpg", false).unwrap();
        b.running.photo_done(Err(Refusal::new(Code::Refused, "no frame")));
        let e = told(&b);
        assert!(matches!(&e[1], Event::Failed { what: Action::Photo, code: Code::Refused, .. }), "{e:?}");
        assert_eq!((e.len(), b.files.info().total), (3, 1));
    }

    /// A recording starts, runs, is stopped as asked and ends once its writer closed the file: `recorded` with it, the
    /// file listed, the controls told it ended before `recorded`. A stop the pipeline could not build leaves it running.
    #[test]
    fn a_recording_stopped_as_asked_ends_with_its_file() {
        let b = bench();
        b.running.record(V1, false).unwrap();
        assert_eq!(*b.record_told.lock().unwrap(), [true]);
        assert_eq!(b.running.record(V1, false).unwrap_err().code, Code::Busy);
        assert_eq!(b.running.record_stop().unwrap_err().code, Code::Busy, "starting");
        b.running.record_built(Ok(()));
        assert_eq!(b.running.record(V1, false).unwrap_err().code, Code::Recording);
        assert_eq!(b.running.play_free().unwrap_err().code, Code::Recording);
        b.running.record_stop().unwrap();
        b.running.record_stop_failed("no".into());
        b.running.record_stop().unwrap();
        b.running.record_stopped();
        assert_eq!(*b.record_told.lock().unwrap(), [true], "stopping until the file is closed");
        assert!(!b.running.record_closed(Ok(V1.into())));
        let e = told(&b);
        assert_eq!(e[..4], [run(rec(V1, RecordState::Starting)), codec(false), run(rec(V1, RecordState::Running)), run(rec(V1, RecordState::Stopping))]);
        assert!(matches!(&e[4], Event::Failed { what: Action::RecordStop, .. }), "{e:?}");
        assert_eq!(e[5..], [run(rec(V1, RecordState::Running)), run(rec(V1, RecordState::Stopping)), Event::Files { version: 1, total: 1 }, codec(true), Event::Recorded { file: V1.into() }, run(Info::default())]);
        assert_eq!(*b.record_told.lock().unwrap(), [true, false]);
        b.running.play_free().unwrap();
    }

    /// Its writer failed (the card full) while it runs: `failed` told once, the recording to be stopped (true, once);
    /// when the pipeline stopped it, it ends with no other outcome.
    #[test]
    fn a_failed_writer_ends_its_recording() {
        let b = bench();
        b.running.record(V1, false).unwrap();
        b.running.record_built(Ok(()));
        told(&b);
        assert!(b.running.record_closed(Err("the card is full".into())));
        let e = told(&b);
        assert!(matches!(&e[..], [Event::Failed { what: Action::Record, code: Code::Refused, .. }]), "{e:?}");
        assert!(!b.running.record_closed(Err("again".into())), "stopped once");
        told(&b);
        b.running.record_stopped();
        assert_eq!(told(&b), [codec(true), run(Info::default())]);
        assert_eq!(*b.record_told.lock().unwrap(), [true, false]);
        b.running.record(V1_2, false).unwrap();
    }

    /// A file closed whole before the stop (the recording restarted into another): listed, not kept as the stop's
    /// outcome — that is the last writer's file.
    #[test]
    fn an_early_close_is_listed_not_the_stops_outcome() {
        let b = bench();
        b.running.record(V1, false).unwrap();
        b.running.record_built(Ok(()));
        told(&b);
        assert!(!b.running.record_closed(Ok(V1.into())));
        assert_eq!(told(&b), [Event::Files { version: 1, total: 1 }]);
        assert_eq!(b.running.play_free().unwrap_err().code, Code::Recording, "still recording");
        b.running.record_stop().unwrap();
        b.running.record_stopped();
        b.running.record_closed(Ok(V1_2.into()));
        assert_eq!(told(&b)[1..], [Event::Files { version: 2, total: 2 }, codec(true), Event::Recorded { file: V1_2.into() }, run(Info::default())]);
    }

    /// Formatting holds the card: no photo, recording, second format or handing to the host until it ends.
    #[test]
    fn formatting_holds_the_card() {
        let b = bench();
        b.running.format(false).unwrap();
        assert_eq!(b.running.photo(P1, false).unwrap_err().code, Code::Formatting);
        assert_eq!(b.running.record(V1, false).unwrap_err().code, Code::Formatting);
        assert_eq!(b.running.format(false).unwrap_err().code, Code::Busy);
        assert_eq!(b.running.card_free().unwrap_err().code, Code::Formatting);
        b.running.format_done(Ok(()));
        assert_eq!(told(&b), [run(Info { format: true, ..Info::default() }), Event::Formatted, run(Info::default())]);
        b.running.card_free().unwrap();
    }

    /// Photos ending while followers subscribe with a snapshot of what runs and of the list: the two take their locks
    /// in one order, so neither waits forever.
    #[test]
    fn photos_ending_beside_snapshots_never_wait_forever() {
        let b = bench();
        let (running, files, followers) = (b.running.clone(), b.files.clone(), b.followers.clone());
        let (done, finished) = std::sync::mpsc::channel();
        let photos = {
            let running = running.clone();
            std::thread::spawn(move || {
                for n in 1..=500 {
                    running.photo(&format!("/mnt/PHOTO/P{n:05}.jpg"), false).unwrap();
                    running.photo_done(Ok(()));
                }
            })
        };
        std::thread::spawn(move || {
            for conn in 2..502 {
                let (c, _rx) = Client::test(conn);
                followers.watch(c, || format!("{:?} {:?}", running.info(), files.info()));
            }
            photos.join().unwrap();
            let _ = done.send(());
        });
        finished.recv_timeout(Duration::from_secs(30)).expect("a photo and a snapshot wait for each other");
        assert_eq!(b.files.info().total, 500);
    }
}
