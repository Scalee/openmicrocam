//! MI_GFX: the 2D engine (GE). No SDK header exists for it here; the structures are rebuilt from the vendor's
//! `mi_gfx.ko` / `libmi_gfx.so` and checked on the camera
//!. Calls queue work on the engine; [`wait`] returns
//! once everything queued is done.
//!
//! Every 1:1 copy into a video frame is GE's ([`copy_nv12`]): it writes exactly the asked columns, where the
//! memory-to-memory scaler writes only whole 16-px ones.
//!
//! One engine for the feed thread and the `ui`, no gate between them: each thread waits only for the last job it queued,
//! through our module `microcam_sys` (kmods/microcam-sys `gewait`), which sleeps on the driver's own queue without mi_gfx's
//! module mutex — mi_gfx's own wait holds that mutex while it sleeps and keeps every other GE call out. The `ui` queues its work in bands of about a
//! millisecond, so a feed's copy waits for at most one of them.

use std::cell::Cell;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use microcam_core::Rect;

use crate::{check, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Fmt {
    Argb4444 = 8,
    Argb8888 = 11,
    /// R, G, B, A bytes in memory: tiny-skia's and libjpeg's order
    Abgr8888 = 14,
    /// two bytes taken as one pixel: an NV12 plane copied as it is (two Y bytes, or one U,V pair, a pixel)
    Rgb565 = 6,
}

impl Fmt {
    pub fn bytes(self) -> u32 {
        match self {
            Fmt::Argb4444 | Fmt::Rgb565 => 2,
            Fmt::Argb8888 | Fmt::Abgr8888 => 4,
        }
    }
}

/// Memory the engine reads or writes: a physical (MIU) address and its layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Surface {
    pub phy: u64,
    pub fmt: Fmt,
    pub w: u32,
    pub h: u32,
    /// bytes per row
    pub stride: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct GRect {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ColourKey {
    on: u8,
    _pad: [u8; 3],
    op: u32,
    fmt: u32,
    start: u32,
    end: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Opt {
    clip: GRect,
    src_key: ColourKey,
    dst_key: ColourKey,
    src_op: u32,
    dst_op: u32,
    mirror: u32,
    rotate: u32,
    flags: u32,
    src_const: u32,
    dst_const: u32,
}

const _: () = assert!(std::mem::size_of::<Surface>() == 24 && std::mem::size_of::<GRect>() == 16 && std::mem::size_of::<Opt>() == 84);

extern "C" {
    fn MI_GFX_Open(dev: i32) -> i32;
    fn MI_GFX_QuickFill(dev: i32, dst: *mut Surface, rect: *mut GRect, colour: u32, fence: *mut u16) -> i32;
    fn MI_GFX_BitBlit(dev: i32, src: *mut Surface, src_rect: *mut GRect, dst: *mut Surface, dst_rect: *mut GRect, opt: *mut Opt, fence: *mut u16) -> i32;
}

/// How a blit treats the destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    /// the source replaces the destination
    Copy,
    /// premultiplied source over premultiplied destination (ONE, INVSRCALPHA)
    OverPremul,
    /// a straight source, premultiplied by the engine on the way (flag SRCPREMULTIPLY), over a premultiplied (or
    /// opaque) destination: the overlay's own ARGB4444 pixels laid over a picture (the bake), exact at a destination
    /// column ≡ 0 (mod 4)
    OverStraight,
    /// the same with the source first scaled by a constant alpha (1…255): its alpha and colour taken times the constant
    /// colour's alpha (flags COLORALPHA | ALPHACHANNEL | SRCPREMULCOL; exact)
    OverPremulAt(u8),
    /// the premultiplied source replaces the destination as straight colour (ONE, ZERO, demultiply)
    Demultiply,
    /// the source's colour with alpha 255 (ONE, ZERO, flag COLORALPHA: the alpha the constant colour's), exact, in
    /// place too — an opaque picture whose writer left its alpha 0
    Opaque,
}

/// Open the engine; once per process (an MI open costs seconds and the last close tears the module down).
pub fn open() -> Result<()> {
    check("MI_GFX_Open", unsafe { MI_GFX_Open(0) })
}

fn grect(r: Rect) -> GRect {
    GRect { x: r.x as i32, y: r.y as i32, w: r.w, h: r.h }
}

/// Fill `r` of `dst` with `argb` (in the surface's own format's terms: straight ARGB8888 bits, converted by the engine).
pub fn fill(dst: &Surface, r: Rect, argb: u32) -> Result<()> {
    let (mut d, mut gr, mut fence) = (*dst, grect(r), 0u16);
    check("MI_GFX_QuickFill", unsafe { MI_GFX_QuickFill(0, &mut d, &mut gr, argb, &mut fence) })?;
    queued(fence);
    Ok(())
}

/// Copy `src_rect` of `src` to (`x`, `y`) of `dst`, the same size, converting the format.
pub fn blit(src: &Surface, src_rect: Rect, dst: &Surface, x: u32, y: u32, blend: Blend) -> Result<()> {
    stretch(src, src_rect, dst, Rect { x, y, ..src_rect }, Rect { x: 0, y: 0, w: dst.w, h: dst.h }, blend)
}

/// Lay `src_rect` of `src` over `dst_rect` of `dst`, stretched when the sizes differ (up-scaling filtered, down-scaling
/// point-sampled), writing only inside `clip`. A destination rectangle starting left of or above its surface is refused
/// by the engine, which then hung the camera: the coordinates are unsigned.
pub fn stretch(src: &Surface, src_rect: Rect, dst: &Surface, dst_rect: Rect, clip: Rect, blend: Blend) -> Result<()> {
    let (mut s, mut d, mut fence) = (*src, *dst, 0u16);
    let (mut sr, mut dr) = (grect(src_rect), grect(dst_rect));
    let (src_op, dst_op, flags, src_const) = match blend {
        Blend::Copy => (11, 11, 0, 0),
        Blend::OverPremul => (1, 5, 0, 0),
        Blend::OverStraight => (1, 5, 0x08, 0),
        Blend::OverPremulAt(a) => (1, 5, 0x13, (a as u32) << 24),
        Blend::Demultiply => (1, 0, 0x80, 0),
        Blend::Opaque => (1, 0, 0x01, 0xff00_0000),
    };
    let mut o = Opt { clip: grect(clip), src_op, dst_op, flags, src_const, ..Default::default() };
    check("MI_GFX_BitBlit", unsafe { MI_GFX_BitBlit(0, &mut s, &mut sr, &mut d, &mut dr, &mut o, &mut fence) })?;
    queued(fence);
    Ok(())
}

thread_local! {
    /// the fence of the last job this thread queued and has not waited for
    static LAST: Cell<Option<u16>> = const { Cell::new(None) };
}

fn queued(fence: u16) {
    LAST.with(|l| l.set(Some(fence)));
}

/// How long a wait may take: the longest job is a whole 4K frame, 13 ms.
const WAIT_MS: u32 = 1000;

/// Everything this thread queued is done (the engine runs its jobs in order: the last one's fence covers them all).
pub fn wait() -> Result<()> {
    let Some(tag) = LAST.with(|l| l.take()) else { return Ok(()) };
    static SYS: OnceLock<std::result::Result<std::fs::File, String>> = OnceLock::new();
    let file = SYS.get_or_init(|| std::fs::OpenOptions::new().write(true).open("/proc/microcam_sys").map_err(|e| e.to_string()));
    let r = match file {
        Ok(f) => (&*f).write_all(format!("gewait {tag} {WAIT_MS}").as_bytes()).map_err(|e| e.to_string()),
        Err(e) => Err(format!("/proc/microcam_sys: {e}")),
    };
    r.map_err(|e| {
        log::warn!("gfx: wait for job {tag}: {e}");
        crate::MiError { call: "microcam_sys gewait", code: -1 }
    })
}

/// An NV12 frame in memory: its planes' physical addresses, the stride both share, and its size.
#[derive(Clone, Copy, Debug)]
pub struct Nv12 {
    pub y: u64,
    pub uv: u64,
    pub stride: u32,
    pub w: u32,
    pub h: u32,
}

impl Nv12 {
    /// Plane `uv` (false: Y) as a GE surface of two-byte pixels.
    fn plane(&self, uv: bool) -> Surface {
        let (phy, h) = if uv { (self.uv, self.h / 2) } else { (self.y, self.h) };
        Surface { phy, fmt: Fmt::Rgb565, w: self.w / 2, h, stride: self.stride }
    }
}

/// `r` of `src` 1:1 to (`x`, `y`) of `dst`, exactly: two blits, the planes taken as two-byte pixels (`r`, `x`, `y`
/// even, as NV12 is).
pub fn copy_nv12(src: &Nv12, r: Rect, dst: &Nv12, x: u32, y: u32) -> Result<()> {
    debug_assert!(r.x % 2 == 0 && r.y % 2 == 0 && r.w % 2 == 0 && r.h % 2 == 0 && x % 2 == 0 && y % 2 == 0, "NV12 copy off the even grid");
    for uv in [false, true] {
        let k = if uv { 2 } else { 1 };
        let sr = Rect { x: r.x / 2, y: r.y / k, w: r.w / 2, h: r.h / k };
        blit(&src.plane(uv), sr, &dst.plane(uv), x / 2, y / k, Blend::Copy)?;
    }
    Ok(())
}

/// `r` of `dst` black (Y 0, U and V 128), exactly.
pub fn black_nv12(dst: &Nv12, r: Rect) -> Result<()> {
    fill(&dst.plane(false), Rect { x: r.x / 2, y: r.y, w: r.w / 2, h: r.h }, 0)?;
    // 0x80, 0x80 as RGB565 bits (R 16 of 31, G 4 of 63): the engine takes the colour as ARGB8888 and keeps the top bits
    fill(&dst.plane(true), Rect { x: r.x / 2, y: r.y / 2, w: r.w / 2, h: r.h / 2 }, 0xff80_1000)
}

static LOST: AtomicBool = AtomicBool::new(false);

/// Whether a wait for the engine has failed: from then on nothing it might still be writing or reading may be freed
/// or reused, and it is not used again.
pub fn lost() -> bool {
    LOST.load(Ordering::SeqCst)
}

pub fn mark_lost() {
    log::error!("gfx: a wait for the 2D engine failed; it is not used again and its memory is kept");
    LOST.store(true, Ordering::SeqCst);
}
