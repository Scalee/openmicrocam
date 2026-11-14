//! The remote: a thread reads its kernel device and sends each key to the scene's owner it was pressed under
//! (`microcam_core::keys`); `key` requests write presses and releases into the same device, so they come back as the
//! remote's own, the kernel's repeats included. A key a connection leaves pressed is released when it closes.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;

use microcam_core::controls::{Code, Refusal};
use microcam_core::keys::{self, Key, Phase, Push, Router, EVENT, EV_KEY, EV_SYN};
use microcam_core::protocol::Event;

use crate::followers::Followers;

/// The remote's input device (rc-core's, «mstar ir»).
const DEVICE: &str = "/dev/input/event0";

pub struct Keys {
    /// the device written into; None when it did not open
    dev: Option<Mutex<File>>,
    /// the keys each connection holds pressed
    held: Mutex<HashMap<u64, BTreeSet<u16>>>,
}

impl Keys {
    /// The device opened for writing and its reader thread started; without it no key comes and `key` is refused.
    pub fn start(followers: Arc<Followers>) -> Arc<Self> {
        let open = OpenOptions::new().read(true).write(true).open(DEVICE).and_then(|w| Ok((w.try_clone()?, w)));
        let dev = match open {
            Ok((r, w)) => match thread::Builder::new().name("keys".into()).spawn(move || read(r, followers)) {
                Ok(_) => Some(Mutex::new(w)),
                Err(e) => {
                    log::error!("keys: no reader thread ({e})");
                    None
                }
            },
            Err(e) => {
                log::error!("keys: {DEVICE}: {e}; no remote");
                None
            }
        };
        Arc::new(Self { dev, held: Mutex::default() })
    }

    /// `key` from connection `conn`: pressed or released in the device.
    pub fn key(&self, conn: u64, key: Key, push: Push) -> Result<(), Refusal> {
        let code = key.code();
        let down = push == Push::Press;
        self.write(code, down).map_err(|e| Refusal::new(Code::Unavailable, format!("key: {e}")))?;
        let mut held = self.held.lock().unwrap();
        let mine = held.entry(conn).or_default();
        if down {
            mine.insert(code);
        } else {
            mine.remove(&code);
        }
        Ok(())
    }

    /// Connection `conn` closed: the keys it left pressed are released.
    pub fn gone(&self, conn: u64) {
        let Some(codes) = self.held.lock().unwrap().remove(&conn) else { return };
        for code in codes {
            if let Err(e) = self.write(code, false) {
                log::warn!("keys: releasing {:?} for connection {conn}: {e}", Key::of(code));
            }
        }
    }

    fn write(&self, code: u16, down: bool) -> Result<(), String> {
        let dev = self.dev.as_ref().ok_or_else(|| format!("{DEVICE} did not open"))?;
        let mut both = [0u8; 2 * EVENT];
        both[..EVENT].copy_from_slice(&keys::event(EV_KEY, code, down as i32));
        both[EVENT..].copy_from_slice(&keys::event(EV_SYN, 0, 0));
        dev.lock().unwrap().write_all(&both).map_err(|e| format!("{DEVICE}: {e}"))
    }
}

/// The reader: each key event named and sent to the owner it was pressed under; an unknown code logged once.
fn read(mut dev: File, followers: Arc<Followers>) {
    let mut router = Router::default();
    let mut unknown = HashSet::new();
    let mut e = [0u8; EVENT];
    loop {
        if let Err(err) = dev.read_exact(&mut e) {
            log::error!("keys: {DEVICE}: {err}; no more keys");
            return;
        }
        let (kind, code, value) = keys::parse(&e);
        let (true, Some(phase)) = (kind == EV_KEY, Phase::of(value)) else { continue };
        let Some(key) = Key::of(code) else {
            if unknown.insert(code) {
                log::warn!("keys: code {code} is no key of the remote; dropped");
            }
            continue;
        };
        followers.to_owner(|owner| router.route(code, phase, owner).then(|| Event::Key { key, phase }));
    }
}
