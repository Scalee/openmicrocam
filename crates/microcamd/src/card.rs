//! The SD card: microcamd owns it. It mounts the card on `/mnt` when it comes in and lets it go when it is pulled
//! (the kernel's block uevents; the vendor's mdev hook is switched off by `app-start.sh`). In the card reader mode (the
//! kept control `card-reader`) it is the USB host's, the gadget's removable mass-storage LUN, whenever it is in its
//! slot — at start, put in, the mode switched on; it comes back when the mode is switched off, before a photo or a
//! recording, or when the host ejects it, and the mode is off then.
//! Photos and recordings go to `PHOTO/` and `VIDEO/` on it (where the vendor's menu listed them). Its space is read
//! when it is mounted and after each change of microcamd's own ([`space_changed`]), never on a clock.

use std::ffi::CString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use microcam_core::controls::{Code, Refusal};
use microcam_core::files::name_number;
use microcam_core::protocol::{Card, CardInfo};
use microcam_core::wish::Wish;

use crate::pipeline::Msg;

pub const CARD: &str = "/mnt";
const BLOCK: &str = "/sys/block/mmcblk0";
const WHOLE: &str = "/dev/mmcblk0";
/// How long a mount waits for mdev to make the device node after the kernel's uevent.
const NODE_WAIT: Duration = Duration::from_secs(2);

/// Told where the card is now (the file list follows it), then what the followers are told.
type Moved = Box<dyn Fn(Card) + Send>;
type Tell = Box<dyn Fn(CardInfo) + Send>;

/// The card reader mode as the controls keep it: read, and switched off when the card leaves the host for the camera
/// or cannot be given to it.
pub struct Mode {
    pub on: Box<dyn Fn() -> bool + Send>,
    pub off: Box<dyn Fn() + Send>,
}

struct Followed {
    at: Card,
    /// the space read at each mount; false until the card's thread runs, which reads it first — at start the first read
    /// after a mount (the FAT counted, about half a second on a 16 GB card) is not waited for
    live: bool,
    moved: Moved,
    tell: Tell,
    mode: Mode,
    tx: Sender<Msg>,
}

static CARD_STATE: Mutex<Option<Followed>> = Mutex::new(None);
/// Where the card is, stored before it is told: read without the card's lock, which is held while it is told (a
/// snapshot read under the followers' lock would otherwise wait for it).
static AT: AtomicU8 = AtomicU8::new(Card::None as u8);
/// The mounted card's space (free, total bytes) as last read, stored before it is told, as `AT`.
static SPACE: Mutex<Option<(u64, u64)>> = Mutex::new(None);

/// `/mnt` is a plain directory on the root file system until the card is mounted there.
pub fn mounted() -> bool {
    std::fs::read_to_string("/proc/mounts").is_ok_and(|m| m.lines().any(|l| l.split(' ').nth(1) == Some(CARD)))
}

/// Why nothing can be written to the card now.
pub fn missing() -> String {
    match now() {
        Card::Host => "the SD card is with the USB host".into(),
        Card::Unreadable => "the camera cannot read the SD card".into(),
        _ => format!("no SD card at {CARD}"),
    }
}

/// The card at the camera from now on: for host tests of what writes to it.
#[cfg(test)]
pub fn here() {
    AT.store(Card::Camera as u8, Ordering::SeqCst);
}

/// Where the card is and, on the camera, its space as last read.
pub fn info() -> CardInfo {
    let at = now();
    let space = if at == Card::Camera { *SPACE.lock().unwrap() } else { None };
    CardInfo { at, free: space.map(|s| s.0), total: space.map(|s| s.1) }
}

/// The card's space changed by something of microcamd's (a file written whole): read again and told when it moved.
pub fn space_changed() {
    if let Some(c) = CARD_STATE.lock().unwrap().as_mut() {
        c.space();
    }
}

pub fn now() -> Card {
    match AT.load(Ordering::SeqCst) {
        x if x == Card::Camera as u8 => Card::Camera,
        x if x == Card::Host as u8 => Card::Host,
        x if x == Card::Unreadable as u8 => Card::Unreadable,
        _ => Card::None,
    }
}

/// Recording number `n` (`Files::next`).
pub fn video(n: u32) -> PathBuf {
    PathBuf::from(format!("{CARD}/VIDEO/V{}.mp4", name_number(n)))
}

pub fn photo(n: u32) -> PathBuf {
    PathBuf::from(format!("{CARD}/PHOTO/P{}.jpg", name_number(n)))
}

/// The card as the facts and the mode put it at start (the host's, mounted by the boot, or mounted now), then its thread
/// following the kernel's events; `moved` gets every move, `tell` every change of the card's state (a move, its space),
/// `tx` is told to stop a recording and the player when the card is pulled.
pub fn start(tx: Sender<Msg>, moved: Moved, tell: Tell, mode: Mode) -> io::Result<()> {
    let sock = uevents()?;
    *CARD_STATE.lock().unwrap() = Some(Followed { at: Card::None, live: false, moved, tell, mode, tx });
    settle();
    thread::Builder::new().name("card".into()).spawn(move || run(sock))?;
    Ok(())
}

/// The card to the USB host (the mode switched on): unmounted here, the gadget's LUN given its block device.
pub fn share() -> Result<(), Refusal> {
    let mut g = CARD_STATE.lock().unwrap();
    let c = g.as_mut().ok_or_else(|| Refusal::new(Code::Refused, "the card is not followed"))?;
    match c.at {
        Card::Host => Ok(()),
        Card::None => Err(Refusal::new(Code::NoCard, missing())),
        Card::Camera | Card::Unreadable => c.give(),
    }
}

/// The card back from the USB host and mounted here (the mode switched off; none to take without a card), or left
/// unmounted when the camera cannot read it (`Unreadable`); refused (`CardOpen`) while the host keeps it open (it locks
/// the medium while it has it mounted).
pub fn take() -> Result<(), Refusal> {
    let refused = |e: String| Refusal::new(Code::Refused, e);
    let mut g = CARD_STATE.lock().unwrap();
    let c = g.as_mut().ok_or_else(|| refused("the card is not followed".into()))?;
    if c.at != Card::Host {
        return Ok(());
    }
    if let Some(lun) = lun_file() {
        std::fs::write(&lun, "\n").map_err(|e| match e.raw_os_error() {
            Some(libc::EBUSY) => Refusal::new(Code::CardOpen, "the USB host keeps the card open: eject it there first"),
            _ => refused(format!("the LUN did not let the card go: {e}")),
        })?;
    }
    match mount() {
        Ok(()) => c.set(Card::Camera),
        Err(e) => {
            log::warn!("card: mount {} on {CARD}: {e}", device().display());
            c.set(Card::Unreadable);
        }
    }
    Ok(())
}

/// The card formatted FAT32 (busybox `mkfs.vfat`, which makes nothing else) and mounted again with `PHOTO/` and `VIDEO/`: `card` told `none` once it is
/// unmounted, `camera` once it is back. The card's lock is held throughout, so its hot-plug thread waits.
pub fn format() -> Result<(), String> {
    let mut g = CARD_STATE.lock().unwrap();
    let c = g.as_mut().ok_or("the card is not followed")?;
    if !matches!(c.at, Card::Camera | Card::Unreadable) {
        return Err(missing());
    }
    if mounted() {
        // SAFETY: plain call.
        unsafe { libc::sync() };
        umount(0).map_err(|e| match e.raw_os_error() {
            Some(libc::EBUSY) => "the card is in use".to_string(),
            _ => format!("unmount {CARD}: {e}"),
        })?;
    }
    c.set(Card::None);
    let dev = device();
    let t0 = Instant::now();
    let made = match std::process::Command::new("mkfs.vfat").arg(&dev).output() {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(format!("mkfs.vfat {}: {}", dev.display(), String::from_utf8_lossy(&o.stderr).trim())),
        Err(e) => Err(format!("mkfs.vfat: {e}")),
    };
    log::info!("card: {} formatted in {} ms: {made:?}", dev.display(), t0.elapsed().as_millis());
    let back = mount().map_err(|e| format!("mount {CARD}: {e}"));
    if back.is_ok() {
        c.set(Card::Camera);
    }
    made?;
    back?;
    for d in ["PHOTO", "VIDEO"] {
        std::fs::create_dir_all(format!("{CARD}/{d}")).map_err(|e| format!("{CARD}/{d}: {e}"))?;
    }
    c.space();
    Ok(())
}

impl Followed {
    /// The card in its slot, mounted here or not, to the USB host: unmounted, the gadget's LUN given its block device;
    /// mounted here when it could not be given.
    fn give(&mut self) -> Result<(), Refusal> {
        let lun = lun_file().ok_or_else(|| Refusal::new(Code::Refused, "the USB gadget has no mass-storage LUN"))?;
        if mounted() {
            // SAFETY: plain call.
            unsafe { libc::sync() };
            umount(0).map_err(|e| match e.raw_os_error() {
                Some(libc::EBUSY) => Refusal::new(Code::Busy, "the card is in use (a recording, a photo or a file playing)"),
                _ => Refusal::new(Code::Refused, format!("unmount {CARD}: {e}")),
            })?;
        }
        // the whole card, its partition table with it, as a card reader shows it
        if let Err(e) = std::fs::write(&lun, WHOLE) {
            let back = match mount() {
                Ok(()) => {
                    self.set(Card::Camera);
                    String::new()
                }
                Err(m) => format!("; mounting it again: {m}"),
            };
            return Err(Refusal::new(Code::Refused, format!("the LUN took no card: {e}{back}")));
        }
        self.set(Card::Host);
        Ok(())
    }

    /// The card given to the USB host by the mode; when it cannot be, mounted here and the mode off.
    fn give_by_mode(&mut self) {
        if let Err(r) = self.give() {
            log::error!("card: not given to the USB host ({}); the card reader mode off", r.error);
            (self.mode.off)();
            if !mounted() && mount().is_ok() {
                self.set(Card::Camera);
            }
        }
    }

    fn set(&mut self, at: Card) {
        if self.at != at {
            log::info!("card: {:?} → {:?}", self.at, at);
            // back from the host (its eject, the mode switched off, a photo or a recording): the mode is off
            if self.at == Card::Host && matches!(at, Card::Camera | Card::Unreadable) {
                (self.mode.off)();
            }
            self.at = at;
            *SPACE.lock().unwrap() = if at == Card::Camera && self.live { space() } else { None };
            AT.store(at as u8, Ordering::SeqCst);
            (self.moved)(at);
            (self.tell)(info());
        }
    }

    /// The mounted card's space read again, told when it moved.
    fn space(&mut self) {
        if self.at != Card::Camera {
            return;
        }
        let now = space();
        let mut stored = SPACE.lock().unwrap();
        if *stored != now {
            *stored = now;
            drop(stored);
            (self.tell)(info());
        }
    }
}

fn run(sock: libc::c_int) {
    if let Some(c) = CARD_STATE.lock().unwrap().as_mut() {
        c.live = true;
        c.space();
    }
    let mut buf = [0u8; 4096];
    loop {
        // SAFETY: `buf` is ours for the call.
        let n = unsafe { libc::recv(sock, buf.as_mut_ptr().cast(), buf.len(), 0) };
        // an mmc block event, or the receive timeout: then only the host's eject is looked for
        if n > 0 && is_card(&buf[..n as usize]) || now() == Card::Host {
            settle();
        }
    }
}

/// Where the facts put the card: in its slot or not, the gadget's LUN holding it or not, mounted here or not.
#[derive(Debug, PartialEq)]
enum Place {
    /// out of its slot: the LUN let go and the mount detached, whichever of them still holds it
    Out,
    /// the host's, by the LUN; mounted here too (`twice`: a second writer on one FAT) it is detached here
    Host { twice: bool },
    /// mounted here
    Camera,
    /// in its slot, nobody's: mounted here
    Free,
}

fn place(present: bool, shared: bool, mounted: bool) -> Place {
    match (present, shared, mounted) {
        (false, _, _) => Place::Out,
        (true, true, twice) => Place::Host { twice },
        (true, false, true) => Place::Camera,
        (true, false, false) => Place::Free,
    }
}

/// The state brought to the facts, never to the state stored: at start it is derived the same way (a restart while the
/// host holds the card leaves it the host's).
fn settle() {
    let mut g = CARD_STATE.lock().unwrap();
    let Some(c) = g.as_mut() else { return };
    let lun = lun_file();
    let shared = lun.as_ref().and_then(|l| std::fs::read_to_string(l).ok()).is_some_and(|f| !f.trim().is_empty());
    match place(Path::new(BLOCK).exists(), shared, mounted()) {
        Place::Out => {
            if c.at == Card::Camera {
                // a recording or a file playing from a card that is gone: stopped (refused when none runs)
                let _ = c.tx.send(Msg::Wish(Wish::RecordStop, None));
                let _ = c.tx.send(Msg::Wish(Wish::PlayStop, None));
            }
            if let (true, Some(l)) = (shared, &lun) {
                let _ = std::fs::write(l, "\n");
            }
            if mounted() {
                if let Err(e) = umount(libc::MNT_DETACH) {
                    log::error!("card: pulled, unmount {CARD}: {e}");
                }
            }
            c.set(Card::None);
        }
        Place::Host { twice } => {
            if twice {
                log::error!("card: the host holds it and {CARD} has it mounted too: unmounted here");
                if let Err(e) = umount(libc::MNT_DETACH) {
                    log::error!("card: unmount {CARD}: {e}");
                }
            }
            c.set(Card::Host);
        }
        // mounted by the boot, or put in, in the card reader mode: the host's
        Place::Camera | Place::Free if c.at != Card::Host && (c.mode.on)() => c.give_by_mode(),
        Place::Camera => c.set(Card::Camera),
        // the host's eject among them: the mode goes off as the card comes back
        Place::Free => match mount() {
            Ok(()) => c.set(Card::Camera),
            Err(e) => {
                log::warn!("card: mount {} on {CARD}: {e}", device().display());
                c.set(Card::Unreadable);
            }
        },
    }
}

/// The mounted card's free and total bytes, None (logged) when they cannot be read.
fn space() -> Option<(u64, u64)> {
    let path = cstr(CARD.as_bytes());
    let t0 = Instant::now();
    // SAFETY: an all-zero statvfs is valid; statvfs fills it from a NUL-terminated path alive for the call.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut st) } != 0 {
        log::warn!("card: space of {CARD}: {}", io::Error::last_os_error());
        return None;
    }
    let unit = st.f_frsize as u64;
    let (free, total) = (st.f_bavail as u64 * unit, st.f_blocks as u64 * unit);
    log::debug!("card: {free} of {total} B free, read in {} ms", t0.elapsed().as_millis());
    Some((free, total))
}

/// The card's first partition, or the whole card when it has none (as the vendor's hot-plug script mounted it).
fn device() -> PathBuf {
    if Path::new(BLOCK).join("mmcblk0p1").exists() {
        PathBuf::from("/dev/mmcblk0p1")
    } else {
        PathBuf::from(WHOLE)
    }
}

fn mount() -> io::Result<()> {
    let dev = device();
    let until = Instant::now() + NODE_WAIT;
    while !dev.exists() && Instant::now() < until {
        thread::sleep(Duration::from_millis(50));
    }
    // the masks named, not taken from the calling thread's umask: every mount of the card the same, as the boot's
    let (src, dst, fs, data) = (cstr(dev.as_os_str().as_encoded_bytes()), cstr(CARD.as_bytes()), cstr(b"vfat"), cstr(b"fmask=0000,dmask=0000"));
    // SAFETY: NUL-terminated strings alive for the call.
    if unsafe { libc::mount(src.as_ptr(), dst.as_ptr(), fs.as_ptr(), 0, data.as_ptr().cast()) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn umount(flags: libc::c_int) -> io::Result<()> {
    let dst = cstr(CARD.as_bytes());
    // SAFETY: a NUL-terminated string alive for the call.
    if unsafe { libc::umount2(dst.as_ptr(), flags) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn cstr(b: &[u8]) -> CString {
    CString::new(b).expect("no NUL in a path")
}

/// The gadget's LUN 0 `file` attribute (the gadget's device beside its controller's).
fn lun_file() -> Option<PathBuf> {
    std::fs::read_dir("/sys/class/udc").ok()?.flatten().map(|e| e.path().join("device/gadget/lun0/file")).find(|p| p.exists())
}

/// The kernel's uevents (group 1), waited for at most a second at a time.
fn uevents() -> io::Result<libc::c_int> {
    // SAFETY: plain calls on a socket we own; the address and the timeout are plain structs.
    unsafe {
        let s = libc::socket(libc::AF_NETLINK, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, libc::NETLINK_KOBJECT_UEVENT);
        if s < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut a: libc::sockaddr_nl = std::mem::zeroed();
        a.nl_family = libc::AF_NETLINK as u16;
        a.nl_groups = 1;
        let t = libc::timeval { tv_sec: 1, tv_usec: 0 };
        if libc::bind(s, (&a as *const libc::sockaddr_nl).cast(), std::mem::size_of_val(&a) as u32) < 0
            || libc::setsockopt(s, libc::SOL_SOCKET, libc::SO_RCVTIMEO, (&t as *const libc::timeval).cast(), std::mem::size_of_val(&t) as u32) < 0
        {
            let e = io::Error::last_os_error();
            libc::close(s);
            return Err(e);
        }
        Ok(s)
    }
}

/// A uevent of the card's block device or a partition on it.
fn is_card(msg: &[u8]) -> bool {
    let mut block = false;
    let mut mmc = false;
    for f in msg.split(|&b| b == 0) {
        block |= f == b"SUBSYSTEM=block";
        mmc |= f.starts_with(b"DEVNAME=mmcblk");
    }
    block && mmc
}

#[cfg(test)]
mod tests {
    use super::{is_card, place, Place};

    #[test]
    fn the_place_follows_the_facts_only() {
        // a restart while the host holds the card: the host's, never mounted here as well
        assert_eq!(place(true, true, false), Place::Host { twice: false });
        assert_eq!(place(true, true, true), Place::Host { twice: true });
        assert_eq!(place(true, false, true), Place::Camera);
        // the host ejected it, or the boot did not mount it
        assert_eq!(place(true, false, false), Place::Free);
        for (shared, mounted) in [(false, false), (false, true), (true, false), (true, true)] {
            assert_eq!(place(false, shared, mounted), Place::Out);
        }
    }

    #[test]
    fn a_card_partition_event_is_the_cards() {
        assert!(is_card(b"add@/devices/x/block/mmcblk0/mmcblk0p1\0ACTION=add\0SUBSYSTEM=block\0DEVNAME=mmcblk0p1\0DEVTYPE=partition\0"));
        assert!(!is_card(b"add@/devices/x/block/mtdblock1\0ACTION=add\0SUBSYSTEM=block\0DEVNAME=mtdblock1\0"));
        assert!(!is_card(b"add@/devices/x/mmc_host/mmc0/mmc0:0001\0ACTION=add\0SUBSYSTEM=mmc\0"));
    }
}
