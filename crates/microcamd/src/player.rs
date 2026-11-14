//! The player: an MP4 on the card shown in the outputs' live windows. The pipeline
//! opens the file at `play` ([`Player::open`]: the video track's index, `microcam_core::demux`) and plans the decoder and
//! its binds; the thread here sends the decoder one sample at a time at the samples' times. Paused, the decoder holds
//! the next sample and decodes it again at the file's rate (`RefreshChn`), so its picture keeps passing the outputs'
//! scaler ports — the menu over it changes as over live, USB keeps its rate. The last sample is not
//! played past: the player pauses on it (`ended`). A seek starts at the sync sample at or before the time asked: playing,
//! that sample is sent next (the one or two already in the ring decode first); paused, the decoder resumes, decodes the
//! held sample and the sync sample, and pauses again on the one after. The ring is never flushed (`vdec::pause`).
//!
//! `play` and `play-stop` are admitted here and answered at once: the player is `starting` or `stopping` before the
//! answer, one of them at a time; the pipeline opens the file and builds, and the outcome — the player `playing`, gone,
//! or `failed` — is told as the state is freed. Admitting, every change and the ending happen under the followers' lock,
//! so the outcome is queued before the next `play` or `play-stop` can be admitted (lock order: `followers`, this state,
//! `Running`'s).

use std::fs::File;
use std::os::unix::fs::FileExt;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use microcam_core::controls::{Code, Refusal};
use microcam_core::demux::{self, Track};
use microcam_core::policy::Playback;
use microcam_core::protocol::{Action, Event, PlayerInfo, PlayerState};
use mi::hal::PlayerControl;
use mi::vdec;

use crate::followers::Followers;

/// How long a send waits for room in the decoder's ring before the thread looks at its commands again.
const SEND_WAIT_MS: i32 = 40;
/// How long a resume is tried while the decoder answers busy (a refresh pending: one decode, ~31 ms at 4K), and a seek
/// waits for the ring to be decoded.
const DECODER_WAIT: Duration = Duration::from_millis(500);
/// How long `stop` waits for the thread: a call into the decoder that never returns leaves it in the kernel.
const STOP_WAIT: Duration = Duration::from_secs(3);

enum Cmd {
    Pause,
    Resume,
    Seek(u64),
    Stop,
}

/// The player as the pipeline and the control see it.
#[derive(Clone)]
pub struct Player(Arc<Shared>);

struct Shared {
    /// the file the pipeline opened last, and its index
    opened: Mutex<Option<(String, Arc<Track>)>>,
    running: Mutex<Option<Running>>,
    st: Mutex<St>,
    /// told every change of what the player shows (`player` events)
    followers: Arc<Followers>,
}

#[derive(Default)]
struct St {
    /// what the thread shows: its file, state and time
    shown: Option<PlayerInfo>,
    /// `play` (its file) or `play-stop` accepted, its outcome not told yet
    busy: Option<Busy>,
}

enum Busy {
    Starting(String),
    Stopping,
}

impl St {
    /// The player as clients see it: starting with its file, what is shown stopping, or what is shown.
    fn view(&self) -> Option<PlayerInfo> {
        match &self.busy {
            Some(Busy::Starting(file)) => Some(PlayerInfo { file: file.clone(), state: PlayerState::Starting, ms: 0, duration_ms: 0 }),
            Some(Busy::Stopping) => self.shown.clone().map(|i| PlayerInfo { state: PlayerState::Stopping, ..i }),
            None => self.shown.clone(),
        }
    }
}

struct Running {
    cmd: Sender<Cmd>,
    /// told by the thread as it ends
    done: Receiver<()>,
    thread: JoinHandle<()>,
}

impl Shared {
    /// `f` changes the state and gives its result and an outcome; under the followers' lock the outcome is told first,
    /// the player after it when its view changed.
    fn change<R>(&self, f: impl FnOnce(&mut St) -> (R, Option<Event>)) -> R {
        let mut r = None;
        self.followers.tell_with(|| {
            let mut st = self.st.lock().unwrap();
            let before = st.view();
            let (res, outcome) = f(&mut st);
            r = Some(res);
            let now = st.view();
            let changed = (now != before).then(|| {
                if let Some(i) = &now {
                    log::info!("player: {} {:?} at {} ms", i.file, i.state, i.ms);
                }
                Event::Player { player: now }
            });
            outcome.into_iter().chain(changed).collect()
        });
        r.expect("the change ran")
    }
}

impl Player {
    pub fn new(followers: Arc<Followers>) -> Self {
        Self(Arc::new(Shared { opened: Mutex::new(None), running: Mutex::new(None), st: Mutex::default(), followers }))
    }

    /// `play {file}`: refused when the file is not there, while the player starts or stops, or when `free` refuses
    /// (a recording, the card being formatted); admitted, the player is `starting` with it.
    pub fn play(&self, file: &str, free: impl FnOnce() -> Result<(), Refusal>) -> Result<(), Refusal> {
        if !std::path::Path::new(file).is_file() {
            return Err(Refusal::new(Code::Invalid, format!("{file}: no such file")));
        }
        self.0.change(|st| {
            let r = Self::idle(st).and_then(|()| free()).map(|()| st.busy = Some(Busy::Starting(file.to_string())));
            (r, None)
        })
    }

    /// The pipeline opened and built the file asked for (the thread sends it), or why not: `failed` told then.
    pub fn play_done(&self, r: Result<(), Refusal>) {
        self.0.change(|st| {
            if !matches!(st.busy, Some(Busy::Starting(_))) {
                return ((), None);
            }
            st.busy = None;
            ((), r.err().map(|r| Event::failed(Action::Play, r)))
        });
    }

    /// `play-stop`: refused when nothing plays, or while the player starts or stops; admitted, it is `stopping`.
    pub fn stop(&self) -> Result<(), Refusal> {
        self.0.change(|st| {
            let r = Self::idle(st).and_then(|()| match st.shown {
                None => Err(Refusal::new(Code::Unavailable, "no file plays")),
                Some(_) => {
                    st.busy = Some(Busy::Stopping);
                    Ok(())
                }
            });
            (r, None)
        })
    }

    /// The player's stop built (its thread gone), or why not: `failed` told then, the player as it was. A stop
    /// `play-stop` did not ask for (the card pulled, the scene's owner gone) frees nothing.
    pub fn stop_done(&self, r: Result<(), Refusal>) {
        self.0.change(|st| {
            if !matches!(st.busy, Some(Busy::Stopping)) {
                return ((), None);
            }
            st.busy = None;
            ((), r.err().map(|r| Event::failed(Action::PlayStop, r)))
        });
    }

    fn idle(st: &St) -> Result<(), Refusal> {
        match st.busy {
            Some(Busy::Starting(_)) => Err(Refusal::new(Code::Busy, "the player is starting")),
            Some(Busy::Stopping) => Err(Refusal::new(Code::Busy, "the player is stopping")),
            None => Ok(()),
        }
    }

    /// `file` (its path) opened and indexed: what the policy plans the decoder from.
    pub fn open(&self, file: &str) -> Result<Playback, String> {
        let path = file.to_string();
        let mut f = File::open(&path).map_err(|e| format!("{path}: {e}"))?;
        let t0 = Instant::now();
        let track = demux::read(&mut f).map_err(|e| format!("{path}: {e}"))?;
        log::info!(
            "player: {path} {:?} {}x{} {} fps, {} samples ({} sync), {} s, largest {} KB, indexed in {} ms",
            track.codec,
            track.size.w,
            track.size.h,
            track.fps(),
            track.samples.len(),
            track.samples.iter().filter(|s| s.sync).count(),
            track.ms(track.duration) / 1000,
            track.largest() / 1024,
            t0.elapsed().as_millis()
        );
        let pb = Playback { path: path.clone(), codec: track.codec, size: track.size, fps: track.fps(), largest: track.largest() as u32 };
        *self.0.opened.lock().unwrap() = Some((path, Arc::new(track)));
        Ok(pb)
    }

    /// The player now, None when no file plays nor starts.
    pub fn info(&self) -> Option<PlayerInfo> {
        self.0.st.lock().unwrap().view()
    }

    pub fn pause(&self) -> Result<(), String> {
        self.command(Cmd::Pause)
    }

    pub fn resume(&self) -> Result<(), String> {
        self.command(Cmd::Resume)
    }

    pub fn seek(&self, ms: u64) -> Result<(), String> {
        self.command(Cmd::Seek(ms))
    }

    fn command(&self, c: Cmd) -> Result<(), String> {
        match self.0.running.lock().unwrap().as_ref() {
            Some(r) => r.cmd.send(c).map_err(|_| "the player has stopped".to_string()),
            None => Err("no file plays".into()),
        }
    }

    /// The Hal's side: the thread started and stopped by the reconciler's steps.
    pub fn control(&self) -> Box<dyn PlayerControl> {
        Box::new(Control(self.0.clone()))
    }
}

struct Control(Arc<Shared>);

impl PlayerControl for Control {
    fn start(&mut self, path: &str) -> Result<(), String> {
        let mut running = self.0.running.lock().unwrap();
        if running.is_some() {
            return Err("the player already sends".into());
        }
        let track = match self.0.opened.lock().unwrap().as_ref() {
            Some((p, t)) if p == path => t.clone(),
            _ => return Err(format!("{path} was not opened")),
        };
        let file = File::open(path).map_err(|e| format!("{path}: {e}"))?;
        let shown = PlayerInfo { file: path.to_string(), state: PlayerState::Playing, ms: 0, duration_ms: track.ms(track.duration) };
        self.0.change(|st| (st.shown = Some(shown), None));
        let (tx, rx) = mpsc::channel();
        let (done_tx, done) = mpsc::channel();
        let shared = self.0.clone();
        let path = path.to_string();
        let thread = thread::Builder::new()
            .name("player".into())
            .spawn(move || {
                Streamer::new(shared, path, track, file).run(rx);
                let _ = done_tx.send(());
            })
            .map_err(|e| e.to_string())?;
        *running = Some(Running { cmd: tx, done, thread });
        Ok(())
    }

    fn stop(&mut self) -> Result<(), String> {
        let Some(r) = self.0.running.lock().unwrap().take() else { return Ok(()) };
        let _ = r.cmd.send(Cmd::Stop);
        if r.done.recv_timeout(STOP_WAIT).is_err() && !r.thread.is_finished() {
            return Err(format!("the player thread did not stop in {} s (a call into the decoder that does not return)", STOP_WAIT.as_secs()));
        }
        r.thread.join().map_err(|_| "the player thread panicked".to_string())?;
        self.0.change(|st| (st.shown = None, None));
        Ok(())
    }
}

struct Streamer {
    shared: Arc<Shared>,
    path: String,
    track: Arc<Track>,
    file: File,
    raw: Vec<u8>,
    au: Vec<u8>,
    /// the sample sent next
    next: usize,
    /// when the sample `next`'s time is due: the instant its time 0 is
    origin: Instant,
    /// paused: held on the sample before `next`; ended: paused on the last sample
    state: PlayerState,
    /// refreshes that failed (not busy)
    missed: u32,
}

impl Streamer {
    fn new(shared: Arc<Shared>, path: String, track: Arc<Track>, file: File) -> Self {
        let largest = track.samples.iter().map(|s| s.size as usize).max().unwrap_or(0);
        let au = Vec::with_capacity(track.largest());
        Self { shared, path, track, file, raw: vec![0; largest], au, next: 0, origin: Instant::now(), state: PlayerState::Playing, missed: 0 }
    }

    fn time(&self, i: usize) -> Duration {
        Duration::from_micros(self.track.samples[i].time * 1_000_000 / self.track.timescale.max(1) as u64)
    }

    fn run(mut self, rx: Receiver<Cmd>) {
        self.told();
        let period = Duration::from_micros(1_000_000 / self.track.fps() as u64);
        let last = self.track.samples.len() - 1;
        loop {
            let wait = match self.state {
                PlayerState::Playing if self.next >= last => {
                    // the last sample: held, not played past
                    self.hold();
                    self.state = PlayerState::Ended;
                    self.told();
                    continue;
                }
                PlayerState::Playing => (self.origin + self.time(self.next)).saturating_duration_since(Instant::now()),
                // paused or ended (starting and stopping are the clients' view, never the thread's)
                _ => period,
            };
            match rx.recv_timeout(wait) {
                Ok(Cmd::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(c) => self.command(c),
                Err(RecvTimeoutError::Timeout) => match self.state {
                    PlayerState::Playing => {
                        if self.send(self.next) {
                            self.next += 1;
                            self.position();
                        }
                    }
                    _ => self.refresh(),
                },
            }
        }
        // the decoder left playing: its channel stops as after a plain play
        if self.state != PlayerState::Playing {
            self.resume();
        }
        log::info!("player: {} stopped at {} ms", self.path, self.track.ms(self.track.samples[self.next.min(last)].time));
    }

    /// The decoder playing again, tried while it answers busy (a refresh pending).
    fn resume(&self) -> bool {
        let until = Instant::now() + DECODER_WAIT;
        loop {
            match vdec::resume() {
                Ok(()) => return true,
                Err(e) if e.code == vdec::BUSY && Instant::now() < until => thread::sleep(Duration::from_millis(2)),
                Err(e) => {
                    log::warn!("player: {e}");
                    return false;
                }
            }
        }
    }

    fn command(&mut self, c: Cmd) {
        match (c, self.state) {
            (Cmd::Pause, PlayerState::Playing) => {
                self.hold();
                self.state = PlayerState::Paused;
            }
            (Cmd::Resume, PlayerState::Paused) => {
                self.resume();
                // the held sample is decoded again now; the next one follows a frame later
                self.origin = Instant::now().checked_sub(self.time(self.next - 1)).unwrap_or_else(Instant::now);
                self.state = PlayerState::Playing;
            }
            (Cmd::Resume, PlayerState::Ended) => {
                self.resume();
                self.seek(0, PlayerState::Playing);
            }
            (Cmd::Seek(ms), s) => self.seek(ms, s),
            _ => return,
        }
        self.told();
    }

    /// Paused where it is: the decoder decodes no new sample, the next one is sent so that it has a sample to decode
    /// again at each refresh.
    fn hold(&mut self) {
        if let Err(e) = vdec::pause() {
            log::warn!("player: {e}");
        }
        while !self.send(self.next) {
            if self.next >= self.track.samples.len() {
                return;
            }
        }
        self.next += 1;
        self.position();
    }

    /// From the sync sample at or before `ms`, in `state` (ended → paused there). Playing, it is the next sample sent.
    /// Paused, the decoder resumes, decodes the held sample and the sync sample (the ring empty), and holds the one after.
    fn seek(&mut self, ms: u64, state: PlayerState) {
        let k = self.track.sync_at(ms);
        self.next = k;
        self.origin = Instant::now().checked_sub(self.time(k)).unwrap_or_else(Instant::now);
        if state != PlayerState::Playing {
            self.resume();
            while !self.send(self.next) {}
            self.next += 1;
            let until = Instant::now() + DECODER_WAIT;
            while vdec::stat().is_ok_and(|(_, _, left)| left > 0) && Instant::now() < until {
                thread::sleep(Duration::from_millis(2));
            }
            if self.next < self.track.samples.len() {
                self.hold();
            }
        }
        self.state = if state == PlayerState::Playing { PlayerState::Playing } else { PlayerState::Paused };
        self.position();
    }

    /// Sample `i` into the decoder; false when the ring had no room in time (try again).
    fn send(&mut self, i: usize) -> bool {
        let Some(s) = self.track.samples.get(i).copied() else { return false };
        let raw = &mut self.raw[..s.size as usize];
        if let Err(e) = self.file.read_exact_at(raw, s.offset) {
            log::error!("player: {} sample {i}: {e}", self.path);
            return true;
        }
        if let Err(e) = self.track.annexb(&s, raw, &mut self.au) {
            log::error!("player: {} sample {i}: {e}", self.path);
            return true;
        }
        // the frame's time on the media clock, as a capture's: the feeds keep their rate by it (the decoder stamps a
        // refreshed frame with that clock too)
        match vdec::send(&self.au, mi::sys::now_pts(), SEND_WAIT_MS) {
            Ok(()) => true,
            Err(e) if e.code == vdec::BUF_FULL => false,
            Err(e) => {
                log::warn!("player: sample {i}: {e}");
                true
            }
        }
    }

    /// A refresh asked; one still pending (`BUSY`: a decode and a scaler pass take about a frame period at 4K) is the
    /// usual answer every other time, a failure is told.
    fn refresh(&mut self) {
        match vdec::refresh() {
            Err(e) if e.code != vdec::BUSY => {
                self.missed += 1;
                if self.missed % 100 == 1 {
                    log::warn!("player: {} refreshes failed, the last {e}", self.missed);
                }
            }
            _ => {}
        }
    }

    fn position(&self) {
        if let Some(i) = self.shared.st.lock().unwrap().shown.as_mut() {
            i.ms = self.track.ms(self.track.samples[self.next.saturating_sub(1)].time);
        }
    }

    fn told(&self) {
        let info = PlayerInfo {
            file: self.path.clone(),
            state: self.state,
            ms: self.track.ms(self.track.samples[self.next.saturating_sub(1)].time),
            duration_ms: self.track.ms(self.track.duration),
        };
        self.shared.change(|st| (st.shown = Some(info), None));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::followers::{events, Client};

    fn shown(file: &str, state: PlayerState) -> PlayerInfo {
        PlayerInfo { file: file.into(), state, ms: 0, duration_ms: 9 }
    }

    /// `play` and `play-stop` are admitted one at a time, the player starting or stopping before the answer; the
    /// outcome — the player playing or gone, or `failed` first and then the player as it is — is told as the state is
    /// freed; a stop nobody asked for frees nothing.
    #[test]
    fn play_and_stop_are_admitted_with_their_state_and_end_with_their_outcome() {
        let followers = Arc::new(Followers::default());
        let (c, rx) = Client::test(1);
        followers.watch(c, String::new);
        let p = Player::new(followers);
        let file = std::env::current_exe().unwrap().to_string_lossy().into_owned();
        let starting = PlayerInfo { file: file.clone(), state: PlayerState::Starting, ms: 0, duration_ms: 0 };
        let player = |i: Option<PlayerInfo>| Event::Player { player: i };

        assert_eq!(p.play("/no/such.mp4", || Ok(())).unwrap_err().code, Code::Invalid);
        assert_eq!(p.play(&file, || Err(Refusal::new(Code::Recording, "r"))).unwrap_err().code, Code::Recording);
        assert_eq!(p.stop().unwrap_err().code, Code::Unavailable, "nothing plays");
        p.play(&file, || Ok(())).unwrap();
        assert_eq!(p.info(), Some(starting.clone()));
        assert_eq!(p.play(&file, || Ok(())).unwrap_err().code, Code::Busy);
        assert_eq!(p.stop().unwrap_err().code, Code::Busy);
        // the thread starts: hidden while starting, shown once the build is told
        p.0.change(|st| (st.shown = Some(shown(&file, PlayerState::Playing)), None));
        p.play_done(Ok(()));
        assert_eq!(events(&rx), [player(Some(starting.clone())), player(Some(shown(&file, PlayerState::Playing)))]);

        p.stop().unwrap();
        assert_eq!(p.play(&file, || Ok(())).unwrap_err().code, Code::Busy);
        p.stop_done(Err(Refusal::new(Code::Refused, "no")));
        let failed = |what| Event::Failed { what, code: Code::Refused, error: "no".into(), holders: Vec::new() };
        assert_eq!(events(&rx), [player(Some(shown(&file, PlayerState::Stopping))), failed(Action::PlayStop), player(Some(shown(&file, PlayerState::Playing)))]);
        p.stop().unwrap();
        p.0.change(|st| (st.shown = None, None));
        p.stop_done(Ok(()));
        assert_eq!(events(&rx), [player(Some(shown(&file, PlayerState::Stopping))), player(None)]);

        p.play(&file, || Ok(())).unwrap();
        p.play_done(Err(Refusal::new(Code::Refused, "no")));
        assert_eq!(events(&rx), [player(Some(starting)), failed(Action::Play), player(None)]);
        p.stop_done(Err(Refusal::new(Code::Refused, "not asked")));
        p.play_done(Ok(()));
        assert_eq!(events(&rx), [], "nothing admitted, nothing freed");
    }
}
