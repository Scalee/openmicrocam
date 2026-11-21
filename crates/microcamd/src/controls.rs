//! The camera's controls in microcamd: the list (`microcam_core::controls`) behind one lock, what a change sets going —
//! a picture knob or a lock goes to the pipeline, which stores it once the ISP took it (at once while no ISP runs) and
//! answers then; a built control goes to the pipeline too, which answers at once and tells once it is built —, the
//! `control` events, and the settings file, which microcamd alone writes.
//!
//! Order: a value is stored before it is told, and an event reads the entry under the followers' lock
//! (`Followers::tell_with`), the lock a snapshot is read and queued under; so a client gets the snapshot, then every
//! change after it (lock order: `followers`).

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use std::collections::BTreeMap;

use microcam_core::controls::{By, Change, Code, Control, Drive, List, Now, Refusal, Saved, Value};
use microcam_core::hdmi::Monitor;
use microcam_core::policy::{Policy, Prefs};
use microcam_core::protocol::{Action, Event, MAX_RECORD};
use microcam_core::tuning::{Knob, Tuning};
use microcam_core::wish::Wish;
use microcam_core::Size;

use crate::pipeline::Msg;
use crate::running::Running;
use crate::followers::Followers;

/// A `set` or `adjust` answered: the value accepted (a built control's at once, to be built; a knob's once the ISP took
/// it), or the refusal.
pub type Reply = Box<dyn FnOnce(Result<Value, Refusal>) + Send>;

/// How long the card reader mode waits for the pipeline's queue before the card goes to the USB host.
const SETTLE: Duration = Duration::from_secs(15);
/// How long the settings stay unchanged before the file is written (a run of changes is saved once).
const QUIET: Duration = Duration::from_millis(1500);

pub struct Controls {
    st: Mutex<State>,
    /// wakes the saver
    changed: Condvar,
    /// one write of the file at a time (the saver's and the one at a clean stop)
    writing: Mutex<()>,
    followers: Arc<Followers>,
    running: Arc<Running>,
    pipeline: Mutex<Sender<Msg>>,
    path: PathBuf,
}

struct State {
    list: List,
    /// the menu's own record (`keep`)
    record: Option<serde_json::Value>,
    /// the file's text as last written or read
    written: String,
    /// when the settings last changed from what the file holds; None: nothing to write
    since: Option<Instant>,
}

impl State {
    fn text(&self) -> String {
        serde_json::to_string_pretty(&Saved { controls: self.list.saved(), record: self.record.clone() }).expect("settings are JSON")
    }
}

impl Controls {
    /// The list with `prefs`' values as defaults and `policy`'s record sizes and HDMI timings, the settings file at `path`
    /// read over them, the built controls put back into `prefs`; its saver started.
    pub fn start(path: PathBuf, prefs: &mut Prefs, policy: &Policy, followers: Arc<Followers>, running: Arc<Running>, pipeline: Sender<Msg>) -> Arc<Self> {
        let mut list = List::new(prefs, policy);
        let saved = read(&path);
        for l in list.load(&saved.controls).into_iter().chain(list.into_prefs(prefs)) {
            log::warn!("{l}");
        }
        let mut st = State { list, record: saved.record, written: String::new(), since: None };
        st.written = st.text();
        let me = Arc::new(Self { st: Mutex::new(st), changed: Condvar::new(), writing: Mutex::new(()), followers, running, pipeline: Mutex::new(pipeline), path });
        let saver = me.clone();
        if let Err(e) = thread::Builder::new().name("settings".into()).spawn(move || saver.saver()) {
            log::error!("settings: no saver ({e}); written at a clean stop only");
        }
        me
    }

    pub fn all(&self) -> Vec<Control> {
        self.st.lock().unwrap().list.all()
    }

    pub fn get(&self, name: &str) -> Option<Control> {
        self.st.lock().unwrap().list.get(name).cloned()
    }

    /// A `set` or `adjust` of `name` asked `by` the user or a USB host (a host's changes only knobs, held while it is
    /// connected: `List::store`); `reply` gets the value accepted or the refusal. A knob or a lock goes to the
    /// pipeline as asked, which resolves it against the list as the changes before it left it and answers once the ISP
    /// took it ([`Msg::Knob`]); a kept value is stored at once; a built control is admitted as being built (one build
    /// of it at a time) and goes to the pipeline, answered at once: its `control` event, or `failed`, tells how it
    /// ended.
    pub fn change(&self, name: &str, change: Change, by: By, reply: Reply) {
        let mut st = self.st.lock().unwrap();
        let drive = st.list.drive(name);
        if let Some(Drive::Knob(_) | Drive::ExposureLock | Drive::WhiteBalanceLock) = drive {
            drop(st);
            let refused = |e: std::sync::mpsc::SendError<Msg>| match e.0 {
                Msg::Knob { reply, .. } => reply(Err(Refusal::new(Code::Refused, "the pipeline thread is gone"))),
                _ => unreachable!("the message sent"),
            };
            let _ = self.pipeline.lock().unwrap().send(Msg::Knob { name: name.to_string(), change, by, reply }).map_err(refused);
            return;
        }
        let v = match st.list.target(name, change) {
            Ok(v) => v,
            Err(r) => {
                drop(st);
                return reply(Err(r));
            }
        };
        // switched on: stored before the card goes, so the host's eject right after it switches it off again;
        // put back when the card did not go. Switched off: stored once the card is back
        if drive == Some(Drive::Card) {
            let was = st.list.get(name).map(|c| c.value.clone());
            drop(st);
            let on = v == Value::Bool(true);
            if on {
                self.edit(|l| l.store(name, v.clone(), by, &Now::default()));
            }
            let r = self.card_reader(on);
            match (&r, was) {
                (Ok(()), _) if !on => self.edit(|l| l.store(name, v.clone(), by, &Now::default())),
                (Err(_), Some(was)) if on => self.edit(|l| l.store(name, was, by, &Now::default())),
                _ => {}
            }
            return reply(r.map(|()| v));
        }
        if drive == Some(Drive::Stored) {
            let told = st.list.store(name, v.clone(), by, &Now::default());
            self.mark(&mut st);
            drop(st);
            self.tell(&told);
            return reply(Ok(v));
        }
        drop(st);
        if let Err(r) = self.running.build(&[name.to_string()]) {
            return reply(Err(r));
        }
        if self.pipeline.lock().unwrap().send(Msg::Control { name: name.to_string(), value: v.clone() }).is_err() {
            self.running.built(Action::Control(name.to_string()), &[name.to_string()], Err(Refusal::new(Code::Refused, "the pipeline thread is gone")));
        }
        reply(Ok(v))
    }

    /// The card reader mode switched: the card given to the USB host — after what the pipeline still has queued (a menu
    /// that just left stops its player there, the file it read open until then) — or taken back.
    fn card_reader(&self, on: bool) -> Result<(), Refusal> {
        self.running.card_free()?;
        if !on {
            return crate::card::take();
        }
        if self.running.info().record.is_some() {
            return Err(Refusal::new(Code::Recording, "a recording is running"));
        }
        let tx = self.pipeline.lock().unwrap().clone();
        crate::pipeline::ask(&tx, Msg::Settle, SETTLE).map_err(|e| Refusal::new(Code::Refused, e))?;
        crate::card::share()
    }

    /// Whether the card reader mode is on.
    pub fn card_reader_on(&self) -> bool {
        self.st.lock().unwrap().list.get("card-reader").is_some_and(|c| c.value == Value::Bool(true))
    }

    /// The list changed by `f` (the names of the entries it changed): told, the file to be written.
    pub fn edit(&self, f: impl FnOnce(&mut List) -> Vec<String>) {
        let mut st = self.st.lock().unwrap();
        let told = f(&mut st.list);
        self.mark(&mut st);
        drop(st);
        self.tell(&told);
    }

    /// What setting `name` to `v` needs read from the running auto first: the exposure, the white balance.
    pub fn reads(&self, name: &str, v: &Value) -> (bool, bool) {
        self.st.lock().unwrap().list.reads(name, v)
    }

    /// Every knob's value for the ISP were the list changed by `f`; nothing stored.
    pub fn tuning_with(&self, f: impl FnOnce(&mut List) -> Vec<String>) -> Tuning {
        let mut l = self.st.lock().unwrap().list.clone();
        f(&mut l);
        l.tuning()
    }

    pub fn drive(&self, name: &str) -> Option<Drive> {
        self.st.lock().unwrap().list.drive(name)
    }

    /// A built control built at the user's `set`: his from now on (the file holds it).
    pub fn chosen(&self, name: &str) {
        let mut st = self.st.lock().unwrap();
        st.list.chosen(name);
        self.mark(&mut st);
    }

    /// The USB host gone (unplugged, or the bus reset): the user's values of what its changes held go back to the ISP
    /// through the pipeline.
    pub fn host_gone(&self) {
        let _ = self.pipeline.lock().unwrap().send(Msg::HostGone);
    }

    /// Whether a USB host's change holds any control now.
    pub fn host_holds(&self) -> bool {
        self.st.lock().unwrap().list.host_holds()
    }

    /// The knobs the ISP refused since its last IQ load: their controls unavailable, told.
    pub fn refused(&self, knobs: BTreeMap<Knob, String>) {
        let told = self.st.lock().unwrap().list.refused(knobs);
        self.tell(&told);
    }

    /// `name`'s value as a USB host reads it: the running auto's when it is live (`now` read for it), else the list's;
    /// Err when it is live and the auto could not be read — the list's manual value is not what the auto runs.
    pub fn read(&self, name: &str, now: impl FnOnce() -> Option<Now>) -> Option<Result<Control, ()>> {
        let mut c = self.get(name)?;
        if self.st.lock().unwrap().list.live(name) {
            match now().and_then(|n| microcam_core::controls::live_value(name, &n)) {
                Some(v) => c.value = v,
                None => return Some(Err(())),
            }
        }
        Some(Ok(c))
    }

    /// `reset`: admitted first, as a build of every built control (refused while any is being built, and no `set` of one
    /// until it ends); the scene taken from the menu by `take_scene` (no `keep` of it after); then every control back to its default —
    /// kept values here, the knobs and locks once the ISP took them and the built ones as one change, both through the
    /// pipeline —, the menu's record gone, the file written at once and again as each part lands.
    pub fn reset(&self, take_scene: impl FnOnce()) -> Result<(), Refusal> {
        let names: Vec<String> = {
            let st = self.st.lock().unwrap();
            st.list.all().into_iter().map(|c| c.name).filter(|n| st.list.drive(n) == Some(Drive::Built)).collect()
        };
        self.running.build(&names)?;
        take_scene();
        let mut st = self.st.lock().unwrap();
        let (told, build) = st.list.reset();
        st.record = None;
        drop(st);
        self.save();
        self.tell(&told);
        let _ = self.pipeline.lock().unwrap().send(Msg::PictureReset);
        if build.is_empty() {
            self.running.built(Action::Reset, &names, Ok(()));
            return Ok(());
        }
        if self.pipeline.lock().unwrap().send(Msg::Reset { build, names: names.clone() }).is_err() {
            self.running.built(Action::Reset, &names, Err(Refusal::new(Code::Refused, "the pipeline thread is gone")));
        }
        Ok(())
    }

    /// The value `change` asks of a control now.
    pub fn target(&self, name: &str, change: Change) -> Result<Value, Refusal> {
        self.st.lock().unwrap().list.target(name, change)
    }

    /// `set` of a built control as a wish to price; refused for a control the pipeline does not build.
    pub fn price_wish(&self, name: &str, value: Value) -> Result<Wish, Refusal> {
        let st = self.st.lock().unwrap();
        if st.list.drive(name).is_some_and(|d| d != Drive::Built) {
            return Err(Refusal::new(Code::Invalid, format!("{name} is not built by the pipeline: nothing to price")));
        }
        Ok(Wish::Control(name.to_string(), st.list.target(name, Change::Set(value))?))
    }

    /// A recording accepted (`on`) or ended (`Running`, under the followers' lock): the record controls' availability
    /// follows at once; the events to tell.
    pub fn record_runs(&self, on: bool) -> Vec<Event> {
        let mut st = self.st.lock().unwrap();
        let told = st.list.record_runs(on);
        told.iter().filter_map(|n| st.list.get(n)).map(|c| Event::Control { control: c.clone() }).collect()
    }

    /// The built controls as the pipeline built the wishes `p`, the HDMI timings
    /// of `monitor`; changes told.
    pub fn follow(&self, p: &Prefs, monitor: &Monitor) {
        let mut st = self.st.lock().unwrap();
        let told = st.list.follow(p, monitor);
        if told.is_empty() {
            return;
        }
        self.mark(&mut st);
        drop(st);
        self.tell(&told);
    }

    /// Every knob's value for the ISP, as the list holds them.
    pub fn tuning(&self) -> Tuning {
        self.st.lock().unwrap().list.tuning()
    }

    /// The knobs' hardware defaults after an IQ load.
    pub fn hardware(&self, values: Vec<(Knob, i32)>) {
        let told = self.st.lock().unwrap().list.hardware(&values);
        self.tell(&told);
    }

    pub fn record_size(&self) -> Size {
        self.st.lock().unwrap().list.record_size()
    }

    /// A `record-size` value's size.
    pub fn record_size_of(&self, v: &Value) -> Option<Size> {
        self.st.lock().unwrap().list.record_size_of(v)
    }

    /// The menu's record kept (run under the followers' lock while the caller is the scene's owner, `Followers::as_menu`).
    pub fn keep(&self, record: serde_json::Value) -> Result<(), String> {
        let len = serde_json::to_string(&record).map_or(usize::MAX, |t| t.len());
        if len > MAX_RECORD {
            return Err(format!("keep: the record is {len} bytes, at most {MAX_RECORD}"));
        }
        let mut st = self.st.lock().unwrap();
        st.record = Some(record);
        self.mark(&mut st);
        Ok(())
    }

    pub fn record(&self) -> Option<serde_json::Value> {
        self.st.lock().unwrap().record.clone()
    }

    /// The file written now if the settings differ from it (a clean stop).
    pub fn save(&self) {
        self.st.lock().unwrap().since = None;
        self.write();
    }

    /// Changes told to the menu and every watcher, each entry as it is when told.
    fn tell(&self, names: &[String]) {
        if names.is_empty() {
            return;
        }
        self.followers.tell_with(|| {
            let st = self.st.lock().unwrap();
            names.iter().filter_map(|n| st.list.get(n)).map(|c| Event::Control { control: c.clone() }).collect()
        });
    }

    /// The saver told when the settings now differ from the file.
    fn mark(&self, st: &mut State) {
        if st.text() != st.written {
            st.since = Some(Instant::now());
            self.changed.notify_one();
        }
    }

    /// Writes the file once the settings have stayed unchanged for [`QUIET`].
    fn saver(&self) {
        let mut st = self.st.lock().unwrap();
        loop {
            let Some(since) = st.since else {
                st = self.changed.wait(st).unwrap();
                continue;
            };
            let left = QUIET.saturating_sub(since.elapsed());
            if !left.is_zero() {
                st = self.changed.wait_timeout(st, left).unwrap().0;
                continue;
            }
            st.since = None;
            drop(st);
            self.write();
            st = self.st.lock().unwrap();
        }
    }

    /// The settings as they are now written, if they differ from the file; the text is read under the write's own
    /// lock, so a later write never carries an older text. A failed write is the saver's again.
    fn write(&self) {
        let _one = self.writing.lock().unwrap();
        let text = {
            let st = self.st.lock().unwrap();
            let text = st.text();
            if text == st.written {
                return;
            }
            text
        };
        match write_atomic(&self.path, text.as_bytes()) {
            Ok(()) => {
                log::info!("settings: {} written", self.path.display());
                self.st.lock().unwrap().written = text;
            }
            Err(e) => {
                // tried again once the settings have stayed for another QUIET, not left until the next change
                log::error!("settings: {}: {e}; tried again", self.path.display());
                let mut st = self.st.lock().unwrap();
                st.since = Some(Instant::now());
                self.changed.notify_one();
            }
        }
    }
}

/// The settings file; missing or unreadable: nothing saved (logged).
fn read(path: &Path) -> Saved {
    match std::fs::read_to_string(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::info!("settings: no {}, the defaults", path.display());
            Saved::default()
        }
        Err(e) => {
            log::warn!("settings: {}: {e}; the defaults", path.display());
            Saved::default()
        }
        Ok(t) => serde_json::from_str(&t).unwrap_or_else(|e| {
            log::warn!("settings: {}: {e}; the defaults", path.display());
            Saved::default()
        }),
    }
}

/// `bytes` as the file at `path`, whole or not at all: written beside it, synced, renamed over it, the directory synced.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut f = File::create(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)?;
    if let Some(dir) = path.parent() {
        File::open(dir)?.sync_all()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_is_replaced_whole_and_read_back() {
        let dir = std::env::temp_dir().join(format!("microcam-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        assert!(read(&path).controls.is_empty(), "missing: the defaults");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(read(&path).controls.is_empty(), "unreadable: the defaults");
        let saved = Saved { controls: [("zoom".to_string(), serde_json::json!(5))].into(), record: Some(serde_json::json!([1, 2])) };
        write_atomic(&path, serde_json::to_string(&saved).unwrap().as_bytes()).unwrap();
        let back = read(&path);
        assert_eq!((back.controls, back.record), (saved.controls, saved.record));
        assert!(!path.with_extension("tmp").exists(), "nothing left beside it");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    use crate::files::Files;
    use crate::followers::{events, Client, Followers};
    use microcam_core::hdmi::Timing;
    use std::sync::mpsc::{channel, Receiver};

    struct Bench {
        controls: Arc<Controls>,
        running: Arc<Running>,
        followers: Arc<Followers>,
        pipeline: Receiver<Msg>,
        prefs: Prefs,
        path: PathBuf,
    }

    /// Controls started over the settings file `saved` (None: no file), in a directory of their own named `name`.
    fn bench(name: &str, saved: Option<&str>) -> Bench {
        let dir = std::env::temp_dir().join(format!("microcam-controls-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let _ = std::fs::remove_file(&path);
        if let Some(t) = saved {
            std::fs::write(&path, t).unwrap();
        }
        let followers = Arc::new(Followers::default());
        let running = Running::new(followers.clone(), Files::new(followers.clone()));
        let (tx, pipeline) = channel();
        let mut prefs = Prefs::default();
        let policy = Policy::parse(include_str!("../../../camera/policy.toml")).unwrap();
        let controls = Controls::start(path.clone(), &mut prefs, &policy, followers.clone(), running.clone(), tx);
        Bench { controls, running, followers, pipeline, prefs, path }
    }

    fn answer() -> (Reply, Receiver<Result<Value, Refusal>>) {
        let (tx, rx) = channel();
        (Box::new(move |r| tx.send(r).unwrap()), rx)
    }

    /// The pipeline's `reset` build, as it would end it.
    fn reset_built(b: &Bench) {
        while let Ok(m) = b.pipeline.try_recv() {
            if let Msg::Reset { names, .. } = m {
                b.running.built(Action::Reset, &names, Ok(()));
                return;
            }
        }
        panic!("no reset build sent");
    }

    /// A `set` of a built control being built: `reset` is refused and nothing of it done — no value, record or scene
    /// touched. A `reset` admitted: no `set` of a built control and no second `reset` until its build ends.
    #[test]
    fn reset_is_admitted_once_against_a_set_being_built() {
        let b = bench("reset", Some(r#"{"controls":{"precedence":"recording > player > hdmi > usb > photo > drawing > picture"}}"#));
        b.controls.keep(serde_json::json!("mine")).unwrap();
        let (reply, got) = answer();
        b.controls.change("zoom", Change::Set(Value::Int(3)), By::User, reply);
        assert_eq!(got.recv().unwrap(), Ok(Value::Int(3)));
        let mut taken = 0;
        assert_eq!(b.controls.reset(|| taken += 1).unwrap_err().code, Code::Busy);
        assert_eq!((taken, b.controls.record()), (0, Some(serde_json::json!("mine"))));
        assert_eq!(b.controls.get("precedence").unwrap().value, Value::Text("recording > player > hdmi > usb > photo > drawing > picture".into()));
        b.running.built(Action::Control("zoom".into()), &["zoom".into()], Ok(()));
        let (watcher, rx) = Client::test(1);
        b.followers.watch(watcher, String::new);
        b.controls.reset(|| taken += 1).unwrap();
        assert_eq!((taken, b.controls.record()), (1, None));
        let (reply, got) = answer();
        b.controls.change("zoom", Change::Set(Value::Int(4)), By::User, reply);
        assert_eq!(got.recv().unwrap().unwrap_err().code, Code::Busy);
        assert_eq!(b.controls.reset(|| taken += 1).unwrap_err().code, Code::Busy);
        reset_built(&b);
        let (reply, got) = answer();
        b.controls.change("zoom", Change::Set(Value::Int(4)), By::User, reply);
        assert_eq!(got.recv().unwrap(), Ok(Value::Int(4)));
        assert!(events(&rx).iter().any(|e| matches!(e, Event::Running { .. })), "the build told");
    }

    /// A USB host reads a live control as the auto runs it; not read, it is not ready — never the list's manual value.
    #[test]
    fn a_live_control_not_read_is_not_the_list_s() {
        let b = bench("live", None);
        let ran = Now { exposure: Some(microcam_core::tuning::ExposureNow { time_100us: 120, gain_pct: 450 }), white_balance: None };
        assert_eq!(b.controls.read("exposure", || Some(ran)).unwrap().unwrap().value, Value::Int(120));
        assert!(b.controls.read("exposure", || None).unwrap().is_err());
        assert!(b.controls.read("white-balance", || Some(ran)).unwrap().is_err(), "the white balance not read");
        assert_eq!(b.controls.read("brightness", || None).unwrap().unwrap().value, Value::Int(50), "not live: the list's");
    }

    /// `reset` leaves the picture's controls as the ISP has them and asks the pipeline for their defaults.
    #[test]
    fn reset_leaves_the_picture_to_the_pipeline() {
        let b = bench("reset-picture", None);
        b.controls.edit(|l| l.store("brightness", Value::Int(70), By::User, &Now::default()));
        b.controls.reset(|| ()).unwrap();
        assert_eq!(b.controls.get("brightness").unwrap().value, Value::Int(70));
        assert!(std::iter::from_fn(|| b.pipeline.try_recv().ok()).any(|m| matches!(m, Msg::PictureReset)));
    }

    /// The scene's owner keeping its record while a `reset` takes the scene: the keep holds the owner while it stores,
    /// so the reset waits for it and clears what it kept; a keep after it is refused.
    #[test]
    fn no_keep_of_the_old_owner_outlives_a_reset() {
        let b = bench("keep", None);
        let (menu, _rx) = Client::test(7);
        b.followers.hello(menu, false, String::new).unwrap();
        let (inside, entered) = channel();
        let (reset_done, after) = channel::<()>();
        let keeper = {
            let (controls, followers) = (b.controls.clone(), b.followers.clone());
            std::thread::spawn(move || {
                // the reset runs meanwhile, unless it waits for this keep
                let kept = followers.as_menu(7, || {
                    inside.send(()).unwrap();
                    let _ = after.recv_timeout(std::time::Duration::from_millis(300));
                    controls.keep(serde_json::json!("old"))
                });
                kept
            })
        };
        entered.recv().unwrap();
        b.controls.reset(|| assert!(b.followers.drop_menu())).unwrap();
        reset_done.send(()).ok();
        assert_eq!(keeper.join().unwrap(), Some(Ok(())));
        assert!(!b.followers.is_menu(7));
        assert_eq!(b.controls.record(), None, "the old owner's keep outlived the reset");
    }

    /// A save waiting while another write runs writes the settings as they are when its turn comes, not as they were
    /// when it asked: an older text is never written after a newer one.
    #[test]
    fn a_waiting_save_writes_the_newest_settings() {
        let b = bench("saver", None);
        let one = b.controls.writing.lock().unwrap();
        let saver = {
            let controls = b.controls.clone();
            std::thread::spawn(move || {
                controls.edit(|l| l.store("brightness", Value::Int(10), By::User, &Now::default()));
                controls.save();
            })
        };
        while b.controls.get("brightness").unwrap().value != Value::Int(10) {
            std::thread::yield_now();
        }
        // the save is waiting for the write
        std::thread::sleep(std::time::Duration::from_millis(50));
        b.controls.edit(|l| l.store("brightness", Value::Int(20), By::User, &Now::default()));
        drop(one);
        saver.join().unwrap();
        let now = b.controls.st.lock().unwrap().text();
        assert!(now.contains("20"));
        assert_eq!(std::fs::read_to_string(&b.path).unwrap(), now);
    }

    /// The knob changes queued to the pipeline, handled in order as it does while no ISP runs: each resolved against the
    /// list as the one before it left it.
    fn knobs_handled(b: &Bench) {
        while let Ok(m) = b.pipeline.try_recv() {
            if let Msg::Knob { name, change, by, reply } = m {
                reply(b.controls.target(&name, change).map(|v| {
                    b.controls.edit(|l| l.store(&name, v.clone(), by, &Now::default()));
                    v
                }));
            }
        }
    }

    /// A held key: two `adjust`s sent before the pipeline handled the first move the value by both; a value written
    /// right after the auto it needs off is not refused for the auto the list still had.
    #[test]
    fn changes_queued_to_the_pipeline_start_from_the_one_before() {
        let b = bench("queued", None);
        let (first, one) = answer();
        let (second, two) = answer();
        b.controls.change("brightness", Change::Adjust(1), By::User, first);
        b.controls.change("brightness", Change::Adjust(1), By::User, second);
        knobs_handled(&b);
        assert_eq!((one.recv().unwrap(), two.recv().unwrap()), (Ok(Value::Int(51)), Ok(Value::Int(52))));
        assert_eq!(b.controls.get("brightness").unwrap().value, Value::Int(52));
        let (off, _off) = answer();
        let (set, got) = answer();
        b.controls.change("auto-exposure", Change::Set(Value::Bool(false)), By::Host, off);
        b.controls.change("exposure", Change::Set(Value::Int(40)), By::Host, set);
        knobs_handled(&b);
        assert_eq!(got.recv().unwrap(), Ok(Value::Int(40)));
        let (unknown, got) = answer();
        b.controls.change("hue", Change::Adjust(1), By::User, unknown);
        assert_eq!(got.recv().unwrap().unwrap_err().code, Code::Unknown, "refused at once");
    }

    /// A write that failed is tried again by the saver, with no change after it.
    #[test]
    fn a_failed_write_is_tried_again() {
        let b = bench("retry", None);
        let dir = b.path.parent().unwrap().to_path_buf();
        std::fs::remove_dir_all(&dir).unwrap();
        b.controls.edit(|l| l.store("brightness", Value::Int(10), By::User, &Now::default()));
        b.controls.save();
        assert!(!b.path.exists());
        std::fs::create_dir_all(&dir).unwrap();
        let until = Instant::now() + QUIET * 3;
        while !b.path.exists() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(std::fs::read_to_string(&b.path).unwrap().contains("\"brightness\": 10"));
    }

    /// A fixed HDMI timing kept in the file is the control's value and the pipeline's wish after the start, though no
    /// monitor offers it yet.
    #[test]
    fn a_kept_hdmi_timing_survives_the_start() {
        let b = bench("hdmi", Some(r#"{"controls":{"hdmi":"1080p60"}}"#));
        assert_eq!(b.controls.get("hdmi").unwrap().value, Value::Text("1080p60".into()));
        assert_eq!((b.prefs.hdmi_enabled, b.prefs.hdmi_override.map(Timing::name)), (true, Some("1080p60")));
    }
}
