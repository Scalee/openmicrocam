//! The connections that follow the camera — the menu, which owns the scene, and the watchers — and the one lock every
//! event to them goes out under. A follower's reply with its snapshot is made and queued under this lock, in the same
//! section it joins the list, and an event that reads the camera's state reads it under this lock too
//! ([`Followers::tell_with`]): so no event after a snapshot says something older than it. Each connection's lines go
//! into its own bounded queue without waiting; a connection that lets it fill is dropped.
//!
//! Lock order: the `ui`'s state, then this list's, then the state a snapshot or an event reads (`Running`'s,
//! `Controls`', `Files`', the player's). A publisher lets its own lock go before it tells, and nothing under this lock
//! takes the `ui`'s.

use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::{Arc, Mutex};

use microcam_core::protocol::Event;

/// A connection that follows the camera (the menu, or a watcher): where its events go, and how it is dropped when it
/// does not read them.
#[derive(Clone)]
pub struct Client {
    pub conn: u64,
    /// the connection's outgoing lines (bounded: video never waits for a slow reader)
    pub tx: SyncSender<String>,
    pub close: Arc<dyn Fn() + Send + Sync>,
}

impl Client {
    pub fn send(&self, ev: &Event) {
        self.queue(serde_json::to_string(ev).unwrap_or_default());
    }

    /// A line onto the connection's queue without waiting; a connection that lets it fill is dropped.
    pub fn queue(&self, line: String) {
        if let Err(TrySendError::Full(_)) = self.tx.try_send(line) {
            log::warn!("followers: connection {} does not read what it is sent; dropped", self.conn);
            (self.close)();
        }
    }
}

#[derive(Default)]
pub struct Followers {
    list: Mutex<List>,
}

#[derive(Default)]
struct List {
    /// the connection that owns the scene
    menu: Option<Client>,
    /// connections that follow the camera without the scene (`watch`)
    watchers: Vec<Client>,
    /// bumped whenever the scene's owner changes (a `hello`, the menu gone or dropped): names the owner a key was
    /// pressed under
    owner: u64,
}

impl List {
    fn is_menu(&self, conn: u64) -> bool {
        self.menu.as_ref().is_some_and(|m| m.conn == conn)
    }

    /// The menu dropped: it hears `lost` when `told`.
    fn drop_menu(&mut self, told: bool) -> bool {
        let Some(m) = self.menu.take() else { return false };
        if told {
            m.send(&Event::Lost);
        }
        self.owner += 1;
        true
    }
}

impl Followers {
    /// `client` a watcher; `reply` made and queued first. Watching twice subscribes once; the menu stays the menu.
    pub fn watch(&self, client: Client, reply: impl FnOnce() -> String) {
        let mut l = self.list.lock().unwrap();
        client.queue(reply());
        if !l.is_menu(client.conn) && !l.watchers.iter().any(|w| w.conn == client.conn) {
            l.watchers.push(client);
        }
    }

    /// `client` the menu, at a new owner's generation: refused while another is the menu, unless `take` — that one
    /// hears `lost`. Accepted, `reply` is made and queued, the connection the menu in the same section. Whether another
    /// menu lost the scene.
    pub fn hello(&self, client: Client, take: bool, reply: impl FnOnce() -> String) -> Result<bool, String> {
        let mut l = self.list.lock().unwrap();
        let other = l.menu.as_ref().filter(|m| m.conn != client.conn).map(|m| m.conn);
        if let (Some(other), false) = (other, take) {
            return Err(format!("connection {other} is the menu (hello with take replaces it)"));
        }
        if let Some(other) = other {
            log::info!("followers: connection {} takes the scene from {other}", client.conn);
        }
        let lost = other.is_some() && l.drop_menu(true);
        client.queue(reply());
        l.watchers.retain(|w| w.conn != client.conn);
        l.menu = Some(client);
        l.owner += 1;
        Ok(lost)
    }

    /// The menu dropped (`reset` takes the scene): it hears `lost`, and is no owner from then on. Whether there was one.
    pub fn drop_menu(&self) -> bool {
        let mut l = self.list.lock().unwrap();
        let conn = l.menu.as_ref().map(|m| m.conn);
        let dropped = l.drop_menu(true);
        if let Some(conn) = conn {
            log::info!("followers: the scene taken from connection {conn}");
        }
        dropped
    }

    /// The connection `conn` closed: it follows nothing more. Whether it was the menu.
    pub fn gone(&self, conn: u64) -> bool {
        let mut l = self.list.lock().unwrap();
        l.watchers.retain(|w| w.conn != conn);
        l.is_menu(conn) && l.drop_menu(false)
    }

    pub fn menu(&self) -> Option<Client> {
        self.list.lock().unwrap().menu.clone()
    }

    /// The menu and the owner's generation it holds the scene at (a new `hello` on the same connection is a new owner).
    pub fn menu_at(&self) -> Option<(Client, u64)> {
        let l = self.list.lock().unwrap();
        l.menu.clone().map(|m| (m, l.owner))
    }

    pub fn is_menu(&self, conn: u64) -> bool {
        self.list.lock().unwrap().is_menu(conn)
    }

    /// `f` run while `conn` is the menu, under this lock, so it cannot be dropped meanwhile; None when it is not the
    /// menu.
    pub fn as_menu<R>(&self, conn: u64, f: impl FnOnce() -> R) -> Option<R> {
        self.list.lock().unwrap().is_menu(conn).then(f)
    }

    /// The menu told `e`.
    pub fn to_menu(&self, e: &Event) {
        if let Some(m) = &self.list.lock().unwrap().menu {
            m.send(e);
        }
    }

    /// The scene's owner told the event `f` makes from who the owner is now (its generation; None: no owner), decided
    /// under the lock an owner changes under.
    pub fn to_owner(&self, f: impl FnOnce(Option<u64>) -> Option<Event>) {
        let l = self.list.lock().unwrap();
        let owner = l.menu.as_ref().map(|_| l.owner);
        if let (Some(e), Some(m)) = (f(owner), &l.menu) {
            m.send(&e);
        }
    }

    /// Every follower told `e` (the menu and the watchers).
    pub fn tell(&self, e: &Event) {
        self.tell_with(|| vec![e.clone()]);
    }

    /// Every follower told the events `f` reads under the lock a snapshot is queued under, so each says what holds when
    /// it goes out.
    pub fn tell_with(&self, f: impl FnOnce() -> Vec<Event>) {
        let l = self.list.lock().unwrap();
        for e in f() {
            for c in l.menu.iter().chain(&l.watchers) {
                c.send(&e);
            }
        }
    }
}

#[cfg(test)]
impl Client {
    /// A client whose lines come out of the receiver; closing it does nothing.
    pub fn test(conn: u64) -> (Self, std::sync::mpsc::Receiver<String>) {
        let (tx, rx) = std::sync::mpsc::sync_channel(1 << 16);
        (Client { conn, tx, close: Arc::new(|| {}) }, rx)
    }
}

/// The events a test client got since it last looked, its non-event lines (replies) skipped.
#[cfg(test)]
pub fn events(rx: &std::sync::mpsc::Receiver<String>) -> Vec<Event> {
    rx.try_iter().filter_map(|l| serde_json::from_str(&l).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A watcher subscribes once however often it watches; `hello` makes it the menu, refused while another is unless
    /// `take`, and the one replaced hears `lost`; the owner's generation moves with each owner.
    #[test]
    fn hello_take_and_lost() {
        let f = Followers::default();
        let (a, ra) = Client::test(1);
        let (b, rb) = Client::test(2);
        f.watch(a.clone(), || "watching".into());
        f.watch(a.clone(), || "again".into());
        f.tell(&Event::Formatted);
        assert_eq!(events(&ra), [Event::Formatted], "subscribed once");
        let gen = |f: &Followers| {
            let mut g = None;
            f.to_owner(|o| {
                g = o;
                None
            });
            g
        };
        assert_eq!(gen(&f), None);
        assert_eq!(f.hello(a.clone(), false, || "hello".into()), Ok(false));
        assert!(f.is_menu(1));
        let first = gen(&f).unwrap();
        assert!(f.hello(b.clone(), false, || unreachable!("refused: no reply")).is_err());
        assert_eq!(f.hello(b.clone(), true, || "hello".into()), Ok(true));
        assert_eq!(events(&ra), [Event::Lost]);
        assert!(gen(&f).unwrap() > first);
        // the old menu follows nothing; the new one is told once, not as a watcher too
        f.tell(&Event::Formatted);
        assert_eq!((events(&ra), events(&rb)), (vec![], vec![Event::Formatted]));
        assert!(f.drop_menu());
        assert_eq!(events(&rb), [Event::Lost]);
        assert!(!f.drop_menu() && gen(&f).is_none());
        assert_eq!(f.hello(b, false, || String::new()), Ok(false));
        assert!(f.gone(2) && f.menu().is_none());
        assert_eq!(events(&rb), [], "a closed menu hears no `lost`");
    }

    /// A connection that lets its queue fill is dropped, and the others still hear.
    #[test]
    fn a_slow_reader_is_dropped() {
        let f = Followers::default();
        let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (tx, _slow) = std::sync::mpsc::sync_channel(1);
        let c = closed.clone();
        f.watch(Client { conn: 1, tx, close: Arc::new(move || c.store(true, std::sync::atomic::Ordering::SeqCst)) }, String::new);
        let (b, rb) = Client::test(2);
        f.watch(b, String::new);
        f.tell(&Event::Formatted);
        assert!(closed.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(events(&rb), [Event::Formatted]);
    }
}
