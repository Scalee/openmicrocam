//! Pictures for the outputs' frames: a JPEG decoded by the CPU (libjpeg-turbo through `jpeg.c`) into an NV12 frame in
//! the media heap, whose parts the memory-to-memory scaler (`MI_SCL_StretchBuf`, its own hardware scaler on SCL dev 3)
//! scales to their windows ([`crate::compose::clean`]). Neither the hardware JPEG decoder (planned for the
//! decode) nor a bound scaler channel is used: a picture channel on SCL dev 1 failed beside a zoomed USB stream.
//!
//! A [`Frame`] is media-heap memory, not a pipeline object: it has no teardown order and freeing it cannot hang, so it
//! is freed when dropped (the no-`Drop` rule is about the latter).

use microcam_core::{Rect, Size};
use mi_sys as ffi;
// the shim calls libjpeg-turbo directly; naming the crate keeps its library in the link
use mozjpeg_sys as _;

use crate::layer::{LayerError, Mma, Use};
use crate::{Result, PIXEL_NV12};

extern "C" {
    fn microcam_jpeg_size(data: *const u8, len: libc::c_ulong, eighths: u32, w: *mut u32, h: *mut u32, ow: *mut u32, oh: *mut u32) -> i32;
    fn microcam_jpeg_nv12(data: *const u8, len: libc::c_ulong, eighths: u32, y: *mut u8, ys: u32, uv: *mut u8, uvs: u32) -> i32;
    fn microcam_jpeg_rgba(data: *const u8, len: libc::c_ulong, eighths: u32, out: *mut u8, os: u32) -> i32;
}

/// A JPEG's own size and its size decoded at `eighths`/8; None when it is not a JPEG the decoder reads.
pub fn jpeg_size(jpeg: &[u8], eighths: u32) -> Option<(Size, Size)> {
    let (mut w, mut h, mut ow, mut oh) = (0, 0, 0, 0);
    // SAFETY: the shim reads `len` bytes of the slice and writes four owned u32s.
    let r = unsafe { microcam_jpeg_size(jpeg.as_ptr(), jpeg.len() as _, eighths, &mut w, &mut h, &mut ow, &mut oh) };
    (r == 0).then(|| (Size::new(w, h), Size::new(ow, oh)))
}

/// A JPEG decoded by the CPU at `eighths`/8 into `out`: opaque RGBA rows of exactly the decoded width (the size
/// [`jpeg_size`] gives).
pub fn jpeg_rgba(jpeg: &[u8], eighths: u32, out: &mut [u8]) -> std::result::Result<(), String> {
    let (_, size) = jpeg_size(jpeg, eighths).ok_or("not a JPEG the decoder reads")?;
    if out.len() < size.w as usize * size.h as usize * 4 {
        return Err(format!("{} bytes for {}x{}", out.len(), size.w, size.h));
    }
    // SAFETY: `out` holds the decoded size's rows at 4 × width bytes each; the shim writes only those.
    if unsafe { microcam_jpeg_rgba(jpeg.as_ptr(), jpeg.len() as _, eighths, out.as_mut_ptr(), size.w * 4) } != 0 {
        return Err("JPEG decode failed".into());
    }
    Ok(())
}

/// An NV12 picture in the media heap: Y plane, then the interleaved Cb/Cr plane, both at `stride`. `size` is the frame's
/// own; `source` is the size of the picture it was decoded from (smaller when decoded scaled), the space crops are given in.
pub struct Frame {
    mem: Mma,
    pub size: Size,
    pub source: Size,
    stride: u32,
    /// where the Cb/Cr plane starts, from the decoded height (`size` may have dropped an odd last line)
    uv: u32,
}

impl Frame {
    /// Decode a JPEG (3-component 4:2:0, what our encoder writes) scaled by `eighths`/8 into a new frame of that size,
    /// out of microcamd's share.
    pub fn decode(jpeg: &[u8], eighths: u32) -> std::result::Result<Frame, String> {
        let (mut src_w, mut src_h, mut w, mut h) = (0, 0, 0, 0);
        // SAFETY: the shim reads `len` bytes of the slice and writes four owned u32s.
        if unsafe { microcam_jpeg_size(jpeg.as_ptr(), jpeg.len() as _, eighths, &mut src_w, &mut src_h, &mut w, &mut h) } != 0 {
            return Err("not a JPEG the decoder reads".into());
        }
        let stride = (w + 15) & !15;
        let rows = (h + 1) & !1;
        let mut f = Frame::counted(Size::new(w, h), Size::new(src_w, src_h), stride, stride * rows * 3 / 2)?;
        let uv = stride * rows;
        // SAFETY: both planes lie inside the mapping (Y: stride × rows, UV: stride × rows/2 after it); the shim writes only
        // rows and columns inside the picture's size.
        let va = f.mem.bytes().as_mut_ptr();
        let r = unsafe { microcam_jpeg_nv12(jpeg.as_ptr(), jpeg.len() as _, eighths, va, stride, va.add(uv as usize), stride) };
        if r != 0 {
            return Err(if r == -2 { "JPEG decode: not 4:2:0".into() } else { "JPEG decode failed".into() });
        }
        // the scaler reads the frame from memory: write the CPU's cached lines back
        f.mem.sync().map_err(|e| e.to_string())?;
        // NV12 for the scaler wants even sizes; an odd scaled size (a non-16:9 picture at 5/8–7/8) drops its last line
        f.size = Size::new(w & !1, h & !1);
        Ok(f)
    }

    /// The hardware decoder's picture brought by SCL dev 3 to `size` (the whole picture) as a new frame out of microcamd's
    /// share; `source` is the JPEG's own size.
    pub fn from_yuyv(y: &crate::jpd::Yuyv, size: Size, source: Size) -> std::result::Result<Frame, String> {
        let (w, h) = (size.w & !1, size.h & !1);
        let stride = (w + 15) & !15;
        let f = Frame::counted(Size::new(w, h), source, stride, stride * h * 3 / 2)?;
        // nothing of the CPU's may be written back over what the scaler writes
        f.mem.sync().map_err(|e| e.to_string())?;
        let all = Rect { x: 0, y: 0, w: y.size.w, h: y.size.h };
        crate::scl::stretch(crate::scl::User::Res, &y.direct(), all, &f.direct()).map_err(|e| e.to_string())?;
        Ok(f)
    }

    /// An opaque picture (`w` × `h` RGBA rows of exactly its width; alpha ignored) as a new frame, out of microcamd's share.
    pub fn from_rgba(w: u32, h: u32, rgba: &[u8]) -> std::result::Result<Frame, String> {
        let (ew, eh) = (w & !1, h & !1);
        if ew == 0 || eh == 0 {
            return Err(format!("{w}x{h}: too small for NV12"));
        }
        let stride = (ew + 15) & !15;
        let mut f = Frame::counted(Size::new(ew, eh), Size::new(w, h), stride, stride * eh * 3 / 2)?;
        let (y, uv, st) = f.planes_mut();
        y.fill(0);
        uv.fill(128);
        // an opaque source written over anything is itself; the blend only converts
        let opaque: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2], 255]).collect();
        microcam_draw::nv12::blend(&opaque, w as usize * 4, ew as usize, eh as usize, y, st, uv, st);
        f.mem.sync().map_err(|e| e.to_string())?;
        Ok(f)
    }

    /// A frame of `size` (width a multiple of 16, height even) out of microcamd's share, its contents undefined: the
    /// scaler fills it.
    pub(crate) fn empty(size: Size) -> std::result::Result<Frame, String> {
        Frame::counted(size, size, size.w, size.w * size.h * 3 / 2)
    }

    /// A picture of the menu's.
    fn counted(size: Size, source: Size, stride: u32, len: u32) -> std::result::Result<Frame, String> {
        Frame::alloc(size, source, stride, len, Use::Menu).map_err(|e| e.to_string())
    }

    /// An NV12 frame at least `size` (even) with undefined contents, for the compositor's clean pictures and patches.
    /// Its width is rounded up to 16: the scaler refuses a source narrower than that (`MI_SCL_IMPL_StretchBuf` checks
    /// an NV12 source's width and stride against 16), so callers copy a crop of it.
    pub(crate) fn blank(size: Size) -> std::result::Result<Frame, LayerError> {
        let stride = (size.w + 15) & !15;
        let size = Size::new(stride, size.h);
        Frame::alloc(size, size, stride, stride * size.h * 3 / 2, Use::Compose)
    }

    /// The Y and the interleaved Cb/Cr plane as the CPU sees them, and their stride. After the hardware wrote the frame,
    /// [`Frame::sync`] first; after the CPU wrote it, [`Frame::sync`] before the hardware reads it.
    pub(crate) fn planes_mut(&mut self) -> (&mut [u8], &mut [u8], usize) {
        let (stride, uv) = (self.stride as usize, self.uv as usize);
        // the Y plane is the first `uv` bytes, the Cb/Cr plane the rest
        let (y, c) = self.mem.bytes().split_at_mut(uv);
        (y, c, stride)
    }

    /// The bytes of microcamd's share it holds.
    pub fn held(&self) -> u64 {
        self.mem.len() as u64
    }

    /// Write the CPU's cached lines back and drop the rest, so the CPU and the hardware see the same memory.
    pub(crate) fn sync(&self) -> Result<()> {
        self.mem.sync()
    }

    fn alloc(size: Size, source: Size, stride: u32, len: u32, usage: Use) -> std::result::Result<Frame, LayerError> {
        // cached: the CPU writes the whole frame row by row
        let mem = Mma::new(len, usage)?;
        let uv = stride * ((size.h + 1) & !1);
        Ok(Frame { mem, size, source, stride, uv })
    }

    /// `crop` of the source picture in this frame's pixels, even-aligned and inside the frame.
    pub(crate) fn part(&self, crop: Rect) -> Rect {
        let at = |v: u32, to: u32, from: u32| (v as u64 * to as u64 / from.max(1) as u64) as u32 & !1;
        let (x, y) = (at(crop.x, self.size.w, self.source.w), at(crop.y, self.size.h, self.source.h));
        let w = at(crop.w, self.size.w, self.source.w).min(self.size.w - x);
        let h = at(crop.h, self.size.h, self.source.h).min(self.size.h - y);
        Rect { x, y, w, h }
    }

    /// The frame as GE's NV12 surface pair.
    pub(crate) fn nv12(&self) -> crate::gfx::Nv12 {
        crate::gfx::Nv12 { y: self.mem.phy, uv: self.mem.phy + self.uv as u64, stride: self.stride, w: self.size.w, h: self.size.h }
    }

    pub(crate) fn direct(&self) -> ffi::MI_SCL_DirectBuf_t {
        ffi::MI_SCL_DirectBuf_t {
            ePixelFormat: PIXEL_NV12,
            u32Width: self.size.w,
            u32Height: self.size.h,
            phyAddr: [self.mem.phy, self.mem.phy + self.uv as u64],
            u32Stride: [self.stride, self.stride],
            u32BuffSize: self.mem.len(),
        }
    }
}
