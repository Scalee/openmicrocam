//! The `ui`'s layers drawn by the GE: the tiles of every output's items kept in the media
//! heap ([`Tiles`]), and per output a [`Composer`] that carries out `microcam_draw::layer`'s plan — the damage composed band
//! by band in a premultiplied scratch, each band copied into the layer demultiplied (straight ARGB, as the overlay
//! hardware blends it). Every call waits for the engine before it returns, so no tile or scratch is in use by the GE
//! once the caller has its result back.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use microcam_core::Rect;
use microcam_draw::layer::{damage, plan};
use microcam_draw::render::{draw_tile, Item, Pixels, Placed, REACH};
use microcam_draw::tiny_skia as sk;
use mi_sys as ffi;

use crate::gfx::{self, Blend, Fmt, Nv12, Surface};
use crate::picture::Frame;
use crate::{check, MiError, Result};

/// What microcamd holds a piece of the media heap for: the heap's own listing names all of microcamd's memory alike
/// (`app-mmaAlloc`), so [`usage`] tells them apart (`microcam mem`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Use {
    /// the menu's tiles and pictures, RGBA and NV12 (the menu budget)
    Menu,
    /// the GE's composition bands
    Scratch,
    /// the ARGB4444 band a bake lays over the still content
    Staging,
    /// the compositor's clean pictures and patches
    Compose,
    /// the menu's RGN canvases (allocated by mi_rgn, charged here before they are made)
    Rgn,
}

impl Use {
    pub const ALL: [Use; 5] = [Use::Menu, Use::Scratch, Use::Staging, Use::Compose, Use::Rgn];

    pub fn name(self) -> &'static str {
        match self {
            Use::Menu => "menu",
            Use::Scratch => "scratch",
            Use::Staging => "staging",
            Use::Compose => "compose",
            Use::Rgn => "rgn",
        }
    }
}

static HELD: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
static PEAK: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];

/// microcamd's share of the media heap — what the video leaves, set by the pipeline after every transition; every use
/// above takes from it — and everything held by every use (the sum of `HELD`): one lock, so a charge is checked
/// against the share as it is when it is made. Of what is held, the spare frames (`T`) are held only ahead of need (the
/// pictures prepared ahead): the book drops them itself when room is needed.
struct Book<T> {
    share: u64,
    total: u64,
    /// the spare, by key, oldest first, and its bytes
    spare: Vec<(u64, T, u64)>,
    spare_bytes: u64,
    /// the highest mark armed ([`wake_at`]): the freed hook is called once what is held firm falls to it
    mark: Option<u64>,
}

impl<T> Book<T> {
    const fn new() -> Self {
        Self { share: u64::MAX, total: 0, spare: Vec::new(), spare_bytes: 0, mark: None }
    }

    fn firm(&self) -> u64 {
        self.total - self.spare_bytes
    }

    /// `n` bytes charged when they fit beside what is held.
    fn take(&mut self, n: u64) -> bool {
        let fits = self.total + n <= self.share;
        if fits {
            self.total += n;
        }
        fits
    }

    /// `n` bytes given back; whether the armed mark is reached (disarmed then).
    fn give(&mut self, n: u64) -> bool {
        self.total -= n;
        self.freed()
    }

    /// `t`, of `bytes` already charged, held as spare under `key`; one held under it before is returned to be dropped,
    /// and whether the armed mark is reached.
    fn spare_put(&mut self, key: u64, t: T, bytes: u64) -> (Option<T>, bool) {
        let old = self.spare_take(key);
        self.spare.push((key, t, bytes));
        self.spare_bytes += bytes;
        (old, self.freed())
    }

    fn spare_take(&mut self, key: u64) -> Option<T> {
        let k = self.spare.iter().position(|(k, ..)| *k == key)?;
        let (_, t, bytes) = self.spare.remove(k);
        self.spare_bytes -= bytes;
        Some(t)
    }

    /// The oldest spare taken out until what is held less them, with `n` more, fits the share: what is taken out, to
    /// be dropped by the caller after the lock (their bytes stay charged until then), and whether it fits after.
    fn make_room(&mut self, n: u64) -> (Vec<T>, bool) {
        let mut out = Vec::new();
        let mut left = self.total;
        while left + n > self.share && !self.spare.is_empty() {
            let (_, t, bytes) = self.spare.remove(0);
            self.spare_bytes -= bytes;
            left -= bytes;
            out.push(t);
        }
        (out, left + n <= self.share)
    }

    fn arm(&mut self, mark: u64) {
        self.mark = Some(self.mark.map_or(mark, |m| m.max(mark)));
    }

    /// Whether the armed mark is reached: disarmed then, the hook to be called by the caller after the lock.
    fn freed(&mut self) -> bool {
        let reached = self.mark.is_some_and(|m| self.firm() <= m);
        if reached {
            self.mark = None;
        }
        reached
    }
}

static BOOK: Mutex<Book<Arc<Frame>>> = Mutex::new(Book::new());

fn book() -> std::sync::MutexGuard<'static, Book<Arc<Frame>>> {
    BOOK.lock().unwrap()
}

/// The share's size; what is held over it stays held, nothing new is taken until it fits again.
pub fn set_share(bytes: u64) {
    book().share = bytes;
}

pub fn share() -> u64 {
    book().share
}

/// What every use holds now, spare frames included.
pub fn held() -> u64 {
    book().total
}

/// What is held for a need: everything but the spare frames ([`spare_put`]), which the book drops when room is needed.
pub fn firm() -> u64 {
    book().firm()
}

/// `frame` held only ahead of need under `key` (a picture prepared ahead): dropped by the book, oldest first, when a
/// charge or an admission needs room; until then [`spare_take`] gives it back.
pub fn spare_put(key: u64, frame: Arc<Frame>) {
    let bytes = frame.held();
    let (old, hook) = book().spare_put(key, frame, bytes);
    drop(old);
    if hook {
        call_freed();
    }
}

/// The spare frame held under `key`, taken out of the spare (held for a need from now on); None when the book dropped it.
pub fn spare_take(key: u64) -> Option<Arc<Frame>> {
    book().spare_take(key)
}

/// What is called once what is held firm falls to the mark armed ([`wake_at`]); set once, by microcamd.
static ON_FREED: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// `f` is called once what is held firm falls to the highest mark armed, from whichever thread freed it, outside the
/// book's lock; it must not block.
pub fn on_freed(f: Box<dyn Fn() + Send + Sync>) {
    let _ = ON_FREED.set(f);
}

/// The freed hook armed for `mark`: called once [`firm`] ≤ the highest mark armed, then disarmed (a waiter whose mark
/// is lower arms again when it looks).
pub fn wake_at(mark: u64) {
    book().arm(mark);
}

fn call_freed() {
    if let Some(f) = ON_FREED.get() {
        f();
    }
}

/// Whether `n` more bytes fit the share beside what is held — when they would not, after the spare frames are dropped,
/// oldest first, until they do. Every charge to the share asks it, and so does whatever decides from the share before
/// charging (the pipeline's admission with 0: the spare beyond the share set to the transition's peak goes).
pub fn room(n: u64) -> bool {
    let (dropped, fits) = book().make_room(n);
    if !dropped.is_empty() {
        log::info!("layer: {} spare frames dropped for room", dropped.len());
    }
    // freed after the lock: dropping a frame gives its bytes back
    drop(dropped);
    fits
}

/// `n` bytes for `usage` out of the share, if they fit beside what is held ([`room`] made when they do not).
fn reserve(usage: Use, n: u64) -> std::result::Result<(), LayerError> {
    if !(book().take(n) || room(n) && book().take(n)) {
        return Err(LayerError::Budget { need: n / 1024, budget: share() / 1024, held: held() / 1024 });
    }
    let h = HELD[usage as usize].fetch_add(n, Ordering::SeqCst) + n;
    PEAK[usage as usize].fetch_max(h, Ordering::SeqCst);
    Ok(())
}

fn release(usage: Use, n: u64) {
    HELD[usage as usize].fetch_sub(n, Ordering::SeqCst);
    let hook = book().give(n);
    if hook {
        call_freed();
    }
}

/// Bytes of the media heap held now and at most since the last [`reset_peaks`], per use.
pub fn usage() -> Vec<(Use, u64, u64)> {
    Use::ALL.iter().map(|&u| (u, HELD[u as usize].load(Ordering::SeqCst), PEAK[u as usize].load(Ordering::SeqCst))).collect()
}

pub fn reset_peaks() {
    for u in Use::ALL {
        PEAK[u as usize].store(HELD[u as usize].load(Ordering::SeqCst), Ordering::SeqCst);
    }
}

/// Media-heap memory mapped into this process, cached; freed when dropped.
pub struct Mma {
    pub phy: u64,
    va: *mut u8,
    len: u32,
    usage: Use,
}

// SAFETY: the mapping is owned by this value alone; shared, it is only read.
unsafe impl Send for Mma {}
unsafe impl Sync for Mma {}

impl Mma {
    /// `len` bytes out of microcamd's share ([`set_share`]); refused when they do not fit beside what is held.
    pub fn new(len: u32, usage: Use) -> std::result::Result<Self, LayerError> {
        reserve(usage, len as u64)?;
        let mut phy: ffi::MI_PHY = 0;
        if let Err(e) = check("MI_SYS_MMA_Alloc", unsafe { ffi::MI_SYS_MMA_Alloc(0, std::ptr::null_mut(), len, &mut phy) }) {
            release(usage, len as u64);
            return Err(e.into());
        }
        let mut va: *mut libc::c_void = std::ptr::null_mut();
        if let Err(e) = check("MI_SYS_Mmap", unsafe { ffi::MI_SYS_Mmap(phy, len, &mut va, 1) }) {
            unsafe { ffi::MI_SYS_MMA_Free(0, phy) };
            release(usage, len as u64);
            return Err(e.into());
        }
        Ok(Self { phy, va: va as *mut u8, len, usage })
    }

    pub fn bytes(&mut self) -> &mut [u8] {
        // SAFETY: `va` maps `len` bytes for as long as this value lives.
        unsafe { std::slice::from_raw_parts_mut(self.va, self.len as usize) }
    }

    pub fn read(&self) -> &[u8] {
        // SAFETY: as in `bytes`.
        unsafe { std::slice::from_raw_parts(self.va, self.len as usize) }
    }

    /// The CPU's cached lines written back and dropped, so the CPU and the engine see the same memory.
    pub fn sync(&self) -> Result<()> {
        check("MI_SYS_FlushInvCache", unsafe { ffi::MI_SYS_FlushInvCache(self.va as *mut _, self.len) })
    }

    pub fn len(&self) -> u32 {
        self.len
    }
}

impl Drop for Mma {
    fn drop(&mut self) {
        unsafe {
            ffi::MI_SYS_Munmap(self.va as *mut _, self.len);
            ffi::MI_SYS_MMA_Free(0, self.phy);
        }
        release(self.usage, self.len as u64);
    }
}

/// Rows of a surface are kept to whole 16-byte steps.
fn stride(w: u32, fmt: Fmt) -> u32 {
    (w * fmt.bytes()).div_ceil(16) * 16
}

/// Bytes of microcamd's share charged while it lives for memory allocated elsewhere (an RGN canvas mi_rgn makes).
pub struct Charge {
    n: u64,
    usage: Use,
}

impl Charge {
    pub fn take(n: u64, usage: Use) -> std::result::Result<Self, LayerError> {
        reserve(usage, n)?;
        Ok(Self { n, usage })
    }
}

impl Drop for Charge {
    fn drop(&mut self) {
        release(self.usage, self.n);
    }
}

/// The menu's media-heap memory; kept for good once the engine is lost (it may still read it).
struct Counted {
    mem: std::mem::ManuallyDrop<Mma>,
}

impl Counted {
    fn new(len: u32) -> std::result::Result<Self, LayerError> {
        Ok(Self { mem: std::mem::ManuallyDrop::new(Mma::new(len, Use::Menu)?) })
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        if !gfx::lost() {
            // SAFETY: dropped once, here
            unsafe { std::mem::ManuallyDrop::drop(&mut self.mem) };
        }
    }
}

/// How much GE writes in about a millisecond, filling a picture (4K ABGR8888 in 24 ms).
const GE_BAND_BYTES: u32 = 1 << 20;

/// A picture in the media heap: premultiplied R, G, B, A, rows of exactly its width — the GE reads it as ABGR8888 and
/// lays it scaled straight from here (`microcam_draw::render::Placed`) — out of microcamd's share.
pub struct Image {
    mem: Counted,
    w: u32,
    h: u32,
}

impl Image {
    pub fn new(w: u32, h: u32) -> std::result::Result<Self, LayerError> {
        Ok(Self { mem: Counted::new(w * h * 4)?, w, h })
    }

    /// Its pixels for the CPU to write; [`Self::sync`] after.
    pub fn bytes(&mut self) -> &mut [u8] {
        self.mem.mem.bytes()
    }

    /// The hardware decoder's picture brought by SCL dev 3 to `w` × `h`, opaque, out of microcamd's share.
    pub fn from_yuyv(y: &crate::jpd::Yuyv, w: u32, h: u32) -> std::result::Result<Self, String> {
        let img = Image::new(w, h).map_err(|e| e.to_string())?;
        // nothing of the CPU's may be written back over what the scaler writes
        img.sync().map_err(|e| e.to_string())?;
        let dst = ffi::MI_SCL_DirectBuf_t {
            ePixelFormat: ffi::MI_SYS_PixelFormat_e::E_MI_SYS_PIXEL_FRAME_ABGR8888,
            u32Width: w,
            u32Height: h,
            phyAddr: [img.mem.mem.phy, 0],
            u32Stride: [w * 4, 0],
            u32BuffSize: w * h * 4,
        };
        let all = microcam_core::Rect { x: 0, y: 0, w: y.size.w, h: y.size.h };
        crate::scl::stretch(crate::scl::User::Res, &y.direct(), all, &dst).map_err(|e| e.to_string())?;
        // the scaler writes alpha 0: GE sets it opaque in place, a band of about a millisecond at a time so a feed's
        // copy waits for at most one
        if gfx::lost() {
            return Err(LayerError::Lost.to_string());
        }
        let s = Surface { phy: img.mem.mem.phy, fmt: Fmt::Abgr8888, w, h, stride: w * 4 };
        let rows = (GE_BAND_BYTES / (w * 4)).max(1);
        for y in (0..h).step_by(rows as usize) {
            let r = microcam_core::Rect { x: 0, y, w, h: rows.min(h - y) };
            gfx::stretch(&s, r, &s, r, r, Blend::Opaque).map_err(|e| e.to_string())?;
            gfx::wait().inspect_err(|_| gfx::mark_lost()).map_err(|e| e.to_string())?;
        }
        // the CPU (tiny-skia) reads it too: no line it may have fetched meanwhile stays
        img.sync().map_err(|e| e.to_string())?;
        Ok(img)
    }

    /// What the CPU wrote, seen by the engine.
    pub fn sync(&self) -> Result<()> {
        self.mem.mem.sync()
    }
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Image {}x{} at {:#x}", self.w, self.h, self.mem.mem.phy)
    }
}

impl Pixels for Image {
    fn size(&self) -> (u32, u32) {
        (self.w, self.h)
    }

    fn rgba(&self) -> &[u8] {
        self.mem.mem.read()
    }

    fn phy(&self) -> Option<u64> {
        Some(self.mem.mem.phy)
    }
}

struct Tile {
    mem: Counted,
    surface: Surface,
    /// the drawing round that last needed it
    used: u64,
}

/// Every output's tiles, by item key: premultiplied RGBA drawn by the CPU straight into the media heap (ABGR8888 to the
/// engine), once, and shared by every output and item that draws the same. A picture laid from its resource has none.
/// Tiles no output needs are dropped oldest first when microcamd's share is reached.
pub struct Tiles {
    map: HashMap<u64, Tile>,
    round: u64,
}

/// Why a scene could not be put in place: it stays as it was.
#[derive(Debug, thiserror::Error)]
pub enum LayerError {
    #[error("microcamd's share of the media heap: {need} KiB more wanted, {held} KiB of {budget} KiB held")]
    Budget { need: u64, budget: u64, held: u64 },
    #[error("a tile: {0}")]
    Raster(String),
    #[error("a picture the 2D engine cannot read")]
    Unreadable,
    #[error("the 2D engine is lost: an earlier wait for it failed")]
    Lost,
    #[error(transparent)]
    Mi(#[from] MiError),
}

impl Tiles {
    pub fn new() -> Self {
        Self { map: HashMap::new(), round: 0 }
    }

    /// Bytes held now by the tiles.
    pub fn bytes(&self) -> u64 {
        self.map.values().map(|t| t.mem.mem.len() as u64).sum()
    }

    /// Make sure every one of `items` has its tile, keeping all of `keep` (what the outputs show now). Room is made by
    /// dropping tiles neither needs, oldest first; when even that is not enough, or the heap refuses, nothing new is kept
    /// and the outputs keep their scenes. Returns the pixels drawn.
    pub fn admit(&mut self, items: &[Item], keep: &HashSet<u64>) -> std::result::Result<u64, LayerError> {
        // tiles the engine may still read are never freed
        if gfx::lost() {
            return Err(LayerError::Lost);
        }
        self.round += 1;
        let round = self.round;
        let mut missing: Vec<&Item> = Vec::new();
        for it in items.iter().filter(|i| i.image.is_none() && !i.hole) {
            match self.map.get_mut(&it.key) {
                Some(t) => t.used = round,
                None if !missing.iter().any(|m| m.key == it.key) => missing.push(it),
                None => {}
            }
        }
        let need: u64 = missing.iter().map(|it| it.tile_size()).map(|(w, h)| w as u64 * h as u64 * 4).sum();
        let wanted: HashSet<u64> = items.iter().map(|i| i.key).collect();
        let mut free: Vec<(u64, u64)> = self.map.iter().filter(|(k, t)| t.used != round && !keep.contains(k) && !wanted.contains(k)).map(|(k, t)| (t.used, *k)).collect();
        free.sort_unstable();
        let freeable: u64 = free.iter().map(|(_, k)| self.map[k].mem.mem.len() as u64).sum();
        let short = || held() + need > share() + freeable;
        if short() {
            // the spare frames go before the scene is refused
            room(need.saturating_sub(freeable));
            if short() {
                return Err(LayerError::Budget { need: need / 1024, budget: share() / 1024, held: held() / 1024 });
            }
        }
        for (_, k) in free {
            if held() + need <= share() {
                break;
            }
            self.map.remove(&k);
        }
        let mut made = Vec::new();
        let mut px = 0;
        for it in missing {
            // the new tiles go with an error: what was there stays
            made.push((it.key, upload(it)?));
            let (w, h) = it.tile_size();
            px += w as u64 * h as u64;
        }
        for (k, t) in made {
            self.map.insert(k, Tile { used: round, ..t });
        }
        Ok(px)
    }

    /// Drop every tile not in `keep` (what the outputs show now): memory is scarcer than the time to draw a tile again.
    pub fn trim(&mut self, keep: &HashSet<u64>) {
        if gfx::lost() {
            return;
        }
        self.map.retain(|k, _| keep.contains(k));
    }
}

/// An item's tile drawn by the CPU straight into the media heap, as the engine reads it (ABGR8888: tiny-skia's R, G, B,
/// A bytes, premultiplied); rows of exactly its width, which the engine takes.
fn upload(it: &Item) -> std::result::Result<Tile, LayerError> {
    let (w, h) = it.tile_size();
    let st = w * 4;
    let mut mem = Counted::new(st * h)?;
    let mut px = sk::PixmapMut::from_bytes(mem.mem.bytes(), w, h).ok_or(LayerError::Raster("an empty tile".into()))?;
    draw_tile(it, &mut px).map_err(LayerError::Raster)?;
    mem.mem.sync()?;
    let surface = Surface { phy: mem.mem.phy, fmt: Fmt::Abgr8888, w, h, stride: st };
    Ok(Tile { mem, surface, used: 0 })
}

/// A band's memory in the scratch, ARGB8888.
const SCRATCH_BYTES: u32 = 2 << 20;
/// Pixels kept free around a band in the scratch: a placed picture's part drawn for a band starts and ends at most a few
/// picture pixels past it ([`lay`]), and the engine refuses a destination rectangle reaching left of or above its surface.
const MARGIN: u32 = (2 * REACH + 4).next_multiple_of(4);

/// The memory every layer's bands are composed in, each band [`MARGIN`] in from its edges: the outputs are drawn one
/// after another by one thread, so one scratch serves all of them. Never freed once the engine is lost.
pub struct Scratch {
    mem: std::mem::ManuallyDrop<Mma>,
    surface: Surface,
    /// the widest band and its most rows
    width: u32,
    rows: u32,
    /// the ARGB4444 band a [`Sink::Bake`] goes through, made on first use
    staging: Option<(std::mem::ManuallyDrop<Mma>, Surface)>,
    /// a bake's still content under a band, NV12 then ARGB8888 ([`Sink::Bake`]), made on first use
    bake: Option<BakeBands>,
}

/// A bake's bands: the still content under a band in NV12 (GE copies it there exactly), then in ARGB8888 (SCL dev 3),
/// where the menu is laid over it. Both as wide as the scratch rounded up to 16 columns: SCL dev 3 reads an NV12 source
/// and writes a destination only in whole 16-column steps.
struct BakeBands {
    nv12: std::mem::ManuallyDrop<Mma>,
    argb: std::mem::ManuallyDrop<Mma>,
    /// columns and rows of both
    w: u32,
    rows: u32,
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if !gfx::lost() {
            // SAFETY: dropped once, here; the engine is done with them (every call waits for it).
            unsafe { std::mem::ManuallyDrop::drop(&mut self.mem) };
            if let Some((m, _)) = self.staging.as_mut() {
                unsafe { std::mem::ManuallyDrop::drop(m) };
            }
            if let Some(b) = self.bake.as_mut() {
                unsafe {
                    std::mem::ManuallyDrop::drop(&mut b.nv12);
                    std::mem::ManuallyDrop::drop(&mut b.argb);
                }
            }
        }
    }
}

impl Scratch {
    /// For layers up to `width` pixels wide.
    pub fn new(width: u32) -> std::result::Result<Self, LayerError> {
        let width = width.max(1);
        let w = width + 2 * MARGIN;
        let st = stride(w, Fmt::Argb8888);
        let rows = (SCRATCH_BYTES / (width * 4)).max(1);
        let h = rows + 2 * MARGIN;
        let mem = Mma::new(st * h, Use::Scratch)?;
        let surface = Surface { phy: mem.phy, fmt: Fmt::Argb8888, w, h, stride: st };
        Ok(Self { mem: std::mem::ManuallyDrop::new(mem), surface, width, rows, staging: None, bake: None })
    }

    /// The widest layer it composes.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The bands a bake goes through given back, made again by the next bake; kept once the engine is lost.
    pub fn unbake(&mut self) {
        if gfx::lost() {
            return;
        }
        if let Some((mut m, _)) = self.staging.take() {
            // SAFETY: taken out, dropped once; the engine is done with it (every call waits for it).
            unsafe { std::mem::ManuallyDrop::drop(&mut m) };
        }
        if let Some(mut b) = self.bake.take() {
            unsafe {
                std::mem::ManuallyDrop::drop(&mut b.nv12);
                std::mem::ManuallyDrop::drop(&mut b.argb);
            }
        }
    }
}

/// The part of a placed picture that draws `clip` (output pixels): its picture pixels over `clip` and one more each side
/// (the filter's neighbours), and where they land by the picture's own scale, rounded — exact wherever the scale is a
/// whole number, within half a pixel of the whole picture's stretch otherwise.
fn lay(p: &Placed, at: (i64, i64), clip: Rect) -> (Rect, (i64, i64, u32, u32)) {
    // along one axis: the clip's start and end, the destination's start and length, the part's start and length
    let axis = |c0: i64, c1: i64, d0: i64, dl: u32, s0: u32, sl: u32| {
        let k = sl as f64 / dl as f64;
        let a = ((s0 as f64 + (c0 - d0) as f64 * k).floor() as i64 - 1).max(s0 as i64);
        let b = ((s0 as f64 + (c1 - d0) as f64 * k).ceil() as i64 + 1).min((s0 + sl) as i64);
        let land = |u: i64| d0 + ((u - s0 as i64) as f64 / k).round() as i64;
        (a, b, land(a), land(b))
    };
    let (dx, dy) = (at.0 + p.dst.0 as i64, at.1 + p.dst.1 as i64);
    let (a, b, x0, x1) = axis(clip.x as i64, (clip.x + clip.w) as i64, dx, p.dst.2, p.src.x, p.src.w);
    let (c, d, y0, y1) = axis(clip.y as i64, (clip.y + clip.h) as i64, dy, p.dst.3, p.src.y, p.src.h);
    (Rect { x: a as u32, y: c as u32, w: (b - a) as u32, h: (d - c) as u32 }, (x0, y0, (x1 - x0) as u32, (y1 - y0) as u32))
}

/// One output's layer brought from scene to scene: what it shows.
#[derive(Default)]
pub struct Composer {
    shown: Vec<Item>,
    /// the layer's content is not known: the next update redraws all of it
    whole: bool,
}

/// What one update did.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub areas: usize,
    pub bands: usize,
    pub blits: usize,
    pub pixels: u64,
}

/// A composer's update drawn a band at a time ([`Composer::begin`], [`Composer::step`]).
pub struct Job {
    items: Vec<Item>,
    area: Rect,
    bands: Vec<microcam_draw::layer::Band>,
    next: usize,
    stats: Stats,
}

impl Job {
    /// The items the layer shows once the job is done.
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// What it drew so far.
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Whether anything is left to draw.
    pub fn is_empty(&self) -> bool {
        self.next >= self.bands.len()
    }
}

impl Composer {
    /// A layer whose content is not known yet: the first update draws all of it.
    pub fn new() -> Self {
        Self { shown: Vec::new(), whole: true }
    }

    /// The items the layer shows.
    pub fn shown(&self) -> &[Item] {
        &self.shown
    }

    /// The layer's content is not what this composer drew (a new layer, or one lost): the next update redraws all of it.
    pub fn reset(&mut self) {
        self.shown.clear();
        self.whole = true;
    }

    /// The layer moved whole by (`dx`, `dy`) in the output frame (its window moved, the content with it): what it shows
    /// is where it was, shifted.
    pub fn moved(&mut self, dx: i64, dy: i64) {
        for i in &mut self.shown {
            i.rect.x = (i.rect.x as i64 + dx).max(0) as u32;
            i.rect.y = (i.rect.y as i64 + dy).max(0) as u32;
        }
    }

    /// Whether bringing a layer that holds `area` to `items` draws anything.
    pub fn needs(&self, items: &[Item], area: Rect) -> bool {
        self.whole || !damage(&self.shown, items, area).is_empty()
    }

    /// Bring the layer to `items`, whose tiles are all in `tiles` ([`Tiles::admit`]). The layer holds `area` of the
    /// output frame the items are placed in. On an engine error the layer's content is unknown and the next call redraws
    /// all of it; when even the wait after it fails, the engine is lost ([`gfx::lost`]): nothing it may still be writing
    /// or reading is freed again, the layer included (its owner checks).
    pub fn show(&mut self, scratch: &mut Scratch, sink: &mut Sink, area: Rect, items: Vec<Item>, tiles: &Tiles) -> std::result::Result<Stats, LayerError> {
        let mut job = self.begin(scratch, matches!(sink, Sink::Bake { .. }), area, items);
        while !self.step(&mut job, scratch, sink, tiles)? {}
        Ok(job.stats)
    }

    /// The update [`Self::show`] would draw, planned to be drawn a band at a time by [`Self::step`] (the
    /// `ui` runs other work between bands). `bake` for a [`Sink::Bake`] (its bands are even).
    pub fn begin(&self, scratch: &Scratch, bake: bool, area: Rect, items: Vec<Item>) -> Job {
        assert!(area.w <= scratch.width, "a layer wider than the scratch");
        let areas = if self.whole { vec![area] } else { damage(&self.shown, &items, area) };
        // the engine's copy into ARGB4444 at a column ≡ 2 or 3 (mod 4) takes its last pixel from one past the source
        // every band starts on a multiple of 4 of the layer
        let areas: Vec<Rect> = areas
            .into_iter()
            .map(|d| {
                let back = (d.x - area.x) % 4;
                Rect { x: d.x - back, w: d.w + back, ..d }
            })
            .collect();
        // one band at a time on the engine, waited for before the next: the feed's copies get the engine between bands
        // SCL dev 3 refuses a band of odd height: a bake's bands are even
        let rows = if bake { (scratch.rows & !1).max(2) } else { scratch.rows };
        let bands = plan(&items, &areas, rows);
        let stats = Stats { areas: areas.len(), bands: bands.len(), ..Stats::default() };
        Job { items, area, bands, next: 0, stats }
    }

    /// The next band of `job` drawn into `sink` (the one the job was begun for); true once every band is drawn — the
    /// layer then shows the job's items. An error as for [`Self::show`]; the job is then over.
    pub fn step(&mut self, job: &mut Job, scratch: &mut Scratch, sink: &mut Sink, tiles: &Tiles) -> std::result::Result<bool, LayerError> {
        if gfx::lost() {
            return Err(LayerError::Lost);
        }
        let Some(b) = job.bands.get(job.next) else {
            self.shown = std::mem::take(&mut job.items);
            self.whole = false;
            return Ok(true);
        };
        if let (Sink::Bake { .. }, None) = (&sink, &scratch.bake) {
            let w = scratch.width.next_multiple_of(16);
            let rows = scratch.rows.next_multiple_of(2);
            let nv12 = Mma::new(w * rows * 3 / 2, Use::Scratch)?;
            let argb = Mma::new(w * 4 * rows, Use::Scratch)?;
            scratch.bake = Some(BakeBands { nv12: std::mem::ManuallyDrop::new(nv12), argb: std::mem::ManuallyDrop::new(argb), w, rows });
        }
        if let (Sink::Bake { .. }, None) = (&sink, &scratch.staging) {
            let st = stride(scratch.width, Fmt::Argb4444);
            let mem = Mma::new(st * scratch.rows, Use::Staging)?;
            let surface = Surface { phy: mem.phy, fmt: Fmt::Argb4444, w: scratch.width, h: scratch.rows, stride: st };
            scratch.staging = Some((std::mem::ManuallyDrop::new(mem), surface));
        }
        let sc = scratch.surface;
        let items = &job.items;
        let area = job.area;
        let mut blits = 0;
        // each item's source: its tile, or its picture (only the band's items are looked up)
        let mut sources: Vec<Option<(Surface, Option<&Placed>)>> = vec![None; items.len()];
        for bl in &b.blits {
            let it = &items[bl.item];
            sources[bl.item] = match &it.image {
                None => tiles.map.get(&it.key).map(|t| (t.surface, None)),
                Some(p) => {
                    let (w, h) = p.pix.size();
                    let phy = p.pix.phy().ok_or(LayerError::Unreadable)?;
                    Some((Surface { phy, fmt: Fmt::Abgr8888, w, h, stride: w * 4 }, Some(p)))
                }
            };
        }
        let r = (|| -> Result<()> {
            let local = Rect { x: MARGIN, y: MARGIN, w: b.rect.w, h: b.rect.h };
            gfx::fill(&sc, local, 0)?;
            for bl in &b.blits {
                let it = &items[bl.item];
                if it.hole {
                    gfx::fill(&sc, Rect { x: MARGIN + bl.x, y: MARGIN + bl.y, w: bl.src.w, h: bl.src.h }, 0)?;
                    continue;
                }
                let blend = match (it.fade.clamp(0.0, 1.0) * 255.0).round() as u8 {
                    0 => continue,
                    255 => Blend::OverPremul,
                    a => Blend::OverPremulAt(a),
                };
                let Some((s, placed)) = &sources[bl.item] else { continue };
                let (x, y) = (MARGIN + bl.x, MARGIN + bl.y);
                match placed {
                    // a flat shape's middle stretched by the engine, exact for a flat source
                    None => {
                        for (from, to) in it.pieces(bl.src) {
                            let (tx, ty) = (x + to.x - bl.src.x, y + to.y - bl.src.y);
                            if from.size() == to.size() {
                                gfx::blit(s, from, &sc, tx, ty, blend)?;
                            } else {
                                let full = Rect { x: 0, y: 0, w: sc.w, h: sc.h };
                                gfx::stretch(s, from, &sc, Rect { x: tx, y: ty, ..to }, full, blend)?;
                            }
                        }
                    }
                    Some(p) => {
                        // in the scratch's pixels: the item's corner, the part of it this band holds
                        let at = (x as i64 - bl.src.x as i64, y as i64 - bl.src.y as i64);
                        let clip = Rect { x, y, ..bl.src };
                        let (part, (dx, dy, dw, dh)) = lay(p, at, clip);
                        debug_assert!(dx >= 0 && dy >= 0, "a placed picture's part beyond the margin");
                        gfx::stretch(s, part, &sc, Rect { x: dx as u32, y: dy as u32, w: dw, h: dh }, clip, blend)?;
                    }
                }
                blits += 1;
            }
            let (x, y) = (b.rect.x - area.x, b.rect.y - area.y);
            match sink {
                Sink::Engine(layer) => {
                    gfx::blit(&sc, local, layer, x, y, Blend::Demultiply)?;
                    gfx::wait()?;
                }
                Sink::Bake { still, patch } => {
                    let (_, ss) = scratch.staging.as_ref().expect("made above");
                    let bb = scratch.bake.as_ref().expect("made above");
                    bake_band(bb, ss, &sc, local, b.rect, still, patch, y)?;
                }
            }
            Ok(())
        })();
        match r {
            Ok(()) => {
                job.stats.blits += blits;
                job.stats.pixels += b.rect.area();
                job.next += 1;
                if job.next == job.bands.len() {
                    self.shown = std::mem::take(&mut job.items);
                    self.whole = false;
                    return Ok(true);
                }
                Ok(false)
            }
            Err(e) => {
                job.next = job.bands.len();
                self.reset();
                if gfx::wait().is_err() {
                    gfx::mark_lost();
                    return Err(LayerError::Lost);
                }
                Err(e.into())
            }
        }
    }
}

/// Where a composer's bands go.
pub enum Sink<'a> {
    /// a surface the engine writes (an RGN region's canvas)
    Engine(Surface),
    /// a baked patch: each band demultiplied into ARGB4444 — the pixels
    /// an RGN canvas would hold, so the menu looks the same over pictures as over the live picture — and laid over
    /// the still content under it (black, then `still` bottom to top) in ARGB8888, then into `patch` (NV12, the
    /// composer's area at its origin; its buffer 16-column wide)
    Bake { still: &'a [Still], patch: Nv12 },
}

/// A picture's part under a bake: `src` holds the picture scaled to `window` (of the output frame), whose first pixel
/// lies at `at` of `src`.
#[derive(Clone, Copy, Debug)]
pub struct Still {
    pub window: Rect,
    pub src: Nv12,
    pub at: (u32, u32),
}

/// An NV12 buffer as SCL dev 3 takes it, `w` × `h` of it from its start.
fn scl_nv12(n: &Nv12, w: u32, h: u32) -> ffi::MI_SCL_DirectBuf_t {
    ffi::MI_SCL_DirectBuf_t { ePixelFormat: crate::PIXEL_NV12, u32Width: w, u32Height: h, phyAddr: [n.y, n.uv], u32Stride: [n.stride, n.stride], u32BuffSize: n.stride * h * 3 / 2 }
}

/// One band of a bake: the still content under `rect` (of the output frame) into the NV12 band by GE, into ARGB8888 by
/// SCL dev 3; the composed band `local` of the scratch `sc` demultiplied into the ARGB4444 `staging` and laid over it;
/// back into rows `y`… of `patch` by SCL dev 3. Waits for the engines.
#[allow(clippy::too_many_arguments)]
fn bake_band(bb: &BakeBands, staging: &Surface, sc: &Surface, local: Rect, rect: Rect, still: &[Still], patch: &Nv12, y: u32) -> Result<()> {
    let (w, h) = (rect.w, rect.h);
    let band = Nv12 { y: bb.nv12.phy, uv: bb.nv12.phy + (bb.w * bb.rows) as u64, stride: bb.w, w: bb.w, h: bb.rows };
    gfx::black_nv12(&band, Rect { x: 0, y: 0, w, h })?;
    for s in still {
        let i = rect.intersect(s.window);
        if i.is_empty() {
            continue;
        }
        let src = Rect { x: s.at.0 + i.x - s.window.x, y: s.at.1 + i.y - s.window.y, ..i };
        gfx::copy_nv12(&s.src, src, &band, i.x - rect.x, i.y - rect.y)?;
    }
    gfx::blit(sc, local, staging, 0, 0, Blend::Demultiply)?;
    gfx::wait()?;
    let argb = |w: u32| ffi::MI_SCL_DirectBuf_t {
        ePixelFormat: ffi::MI_SYS_PixelFormat_e::E_MI_SYS_PIXEL_FRAME_ARGB8888,
        u32Width: w,
        u32Height: h,
        phyAddr: [bb.argb.phy, 0],
        u32Stride: [bb.w * 4, 0],
        u32BuffSize: bb.w * 4 * h,
    };
    let crop = Rect { x: 0, y: 0, w, h };
    crate::scl::stretch(crate::scl::User::Ui, &scl_nv12(&band, bb.w, h), crop, &argb(w))?;
    let a = Surface { phy: bb.argb.phy, fmt: Fmt::Argb8888, w: bb.w, h, stride: bb.w * 4 };
    gfx::blit(staging, crop, &a, 0, 0, Blend::OverStraight)?;
    gfx::wait()?;
    let to = Nv12 { y: patch.y + (y * patch.stride) as u64, uv: patch.uv + (y / 2 * patch.stride) as u64, ..*patch };
    crate::scl::stretch(crate::scl::User::Ui, &argb(bb.w), crop, &scl_nv12(&to, w, h))
}

/// The menu `items` (their tiles in `tiles`) baked over the still content — black, then `still` bottom to top — over
/// `rect` of the output frame (even edges): a new NV12 frame of `rect`'s size, the menu in it as an RGN canvas shows it.
/// On an error the engine may be lost ([`gfx::lost`]), as for a layer.
pub fn bake(scratch: &mut Scratch, rect: Rect, items: &[Item], tiles: &Tiles, still: &[Still]) -> std::result::Result<crate::picture::Frame, LayerError> {
    let mut b = BakeJob::begin(scratch, rect, items)?;
    while !b.step(scratch, tiles, still)? {}
    Ok(b.patch)
}

/// A [`bake`] drawn a band at a time: the `ui` runs other work between bands.
pub struct BakeJob {
    rect: Rect,
    patch: crate::picture::Frame,
    composer: Composer,
    job: Job,
}

impl BakeJob {
    /// The bake of `items` over `rect` (even edges), planned; its frame made.
    pub fn begin(scratch: &Scratch, rect: Rect, items: &[Item]) -> std::result::Result<Self, LayerError> {
        debug_assert!(rect.x % 2 == 0 && rect.y % 2 == 0 && rect.w % 2 == 0 && rect.h % 2 == 0, "a patch off the even grid");
        let patch = crate::picture::Frame::blank(rect.size())?;
        let composer = Composer::new();
        let on: Vec<Item> = items.iter().filter(|i| !i.rect.intersect(rect).is_empty()).cloned().collect();
        let job = composer.begin(scratch, true, rect, on);
        Ok(Self { rect, patch, composer, job })
    }

    /// The next band baked over `still`; true once the patch is whole ([`Self::patch`]).
    pub fn step(&mut self, scratch: &mut Scratch, tiles: &Tiles, still: &[Still]) -> std::result::Result<bool, LayerError> {
        self.composer.step(&mut self.job, scratch, &mut Sink::Bake { still, patch: self.patch.nv12() }, tiles)
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// The tiles it draws from (kept while it runs).
    pub fn keys(&self) -> impl Iterator<Item = u64> + '_ {
        self.job.items().iter().map(|i| i.key)
    }

    /// The patch, once [`Self::step`] said it is whole.
    pub fn patch(self) -> crate::picture::Frame {
        self.patch
    }
}

#[cfg(test)]
mod share_tests {
    use super::*;

    /// Room is made by taking the spare out oldest first, only as much as the charge needs; with no spare left a charge
    /// that does not fit is refused, and nothing is taken out for one that fits. What is held firm leaves the spare out.
    #[test]
    fn room_drops_the_oldest_spare_first_and_only_what_is_needed() {
        let mut b = Book::new();
        b.share = 100;
        assert!(b.take(90));
        assert!(b.spare_put(1, "a", 10).0.is_none());
        b.spare_put(2, "b", 20);
        b.spare_put(3, "c", 30);
        assert_eq!(b.firm(), 30);
        assert_eq!(b.make_room(10), (vec![], true), "fits: nothing taken");
        assert_eq!(b.make_room(15), (vec!["a"], true));
        b.give(10);
        assert_eq!((b.firm(), b.spare_bytes), (30, 50));
        assert_eq!(b.spare_put(3, "c2", 30).0, Some("c"), "a key put again: the one before goes");
        assert_eq!(b.spare_take(2), Some("b"));
        assert_eq!(b.spare_take(2), None, "taken once");
        assert_eq!(b.make_room(80), (vec!["c2"], false), "all the spare out, still short");
        assert_eq!(b.spare_bytes, 0);
    }

    /// The freed mark: reached once what is held firm falls to the highest mark armed, then disarmed; a frame put into
    /// the spare frees as a release does.
    #[test]
    fn the_freed_mark_is_reached_once_at_the_highest_mark() {
        let mut b: Book<&str> = Book::new();
        b.share = 100;
        assert!(b.take(90));
        b.arm(40);
        b.arm(70);
        assert!(!b.give(10), "80 firm, the mark 70");
        assert!(b.spare_put(1, "a", 10).1, "70: the higher mark reached");
        assert!(!b.give(10), "disarmed: the lower waiter arms again");
        b.arm(40);
        assert!(!b.give(10));
        assert!(b.give(10), "40");
    }
}
