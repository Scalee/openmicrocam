//! MI_JPD: the hardware JPEG decoder (NJPD) for the menu's pictures.
//!
//! A JPEG is read into a media-heap block of microcamd's share and handed to `MI_JPD_DirectBufDecode` by its physical
//! address, so the kernel makes no block of its own; the engine writes YUYV straight into a buffer of ours, scaled down
//! on the way by 1, 2, 4 or 8. The input is padded with zero bytes to a multiple of 16: mhal ends the engine's read
//! window at the input's exact last byte, and with that 16-byte unit holding 1–8 bytes the engine may stop short of its
//! last bits and never finish. The engine's scale is set by a
//! channel's maximum resolution, so a channel is made for each decode (3.7 ms) and destroyed after it. One decode at a
//! time, on channel 0: the `res` thread's pictures and a photo's thumbnail.

use microcam_core::Size;
use mi_sys as ffi;

use crate::layer::{Mma, Use};
use crate::check;

const DEV: u32 = 0;
const CHN: u32 = 0;
/// The channel's input ring (`MI_JPD_GetStreamBuf`), unused by a direct decode; the smallest the module takes.
const RING: u32 = 0x1000;
/// The maximum that leaves a picture at its own size (the module's limit).
const WHOLE: u32 = 8192;
/// `E_MI_SYS_PIXEL_FRAME_YUV422_YUYV`: what the engine writes.
const PIXEL_YUYV: u32 = ffi::MI_SYS_PixelFormat_e::E_MI_SYS_PIXEL_FRAME_YUV422_YUYV;

// libmi_jpd's own types; no SDK header declares them (layouts checked on the camera 2026-11-11).
#[repr(C)]
struct ChnAttr {
    stream_buf_size: u32,
    codec: u32,
    max_w: u32,
    max_h: u32,
}

#[repr(C)]
struct DirectIn {
    va: *const u8,
    phy: u64,
    len: u32,
}

#[repr(C)]
struct DirectOut {
    format: u32,
    w: u32,
    h: u32,
    stride: [u32; 2],
    phy: [u64; 2],
}

/// `QueryStreamInfo`'s answer: the picture's own size, then the buffer it decodes into (MCU-aligned, scaled).
#[repr(C)]
#[derive(Default)]
struct StreamInfo {
    w: u32,
    h: u32,
    /// not written by the module
    format: u32,
    buf_w: u32,
    buf_h: u32,
}

extern "C" {
    fn MI_JPD_CreateChn(dev: u32, chn: u32, attr: *mut ChnAttr) -> i32;
    fn MI_JPD_DestroyChn(dev: u32, chn: u32) -> i32;
    fn MI_JPD_StartChn(dev: u32, chn: u32) -> i32;
    fn MI_JPD_StopChn(dev: u32, chn: u32) -> i32;
    fn MI_JPD_QueryStreamInfo(dev: u32, chn: u32, input: *mut DirectIn, info: *mut StreamInfo) -> i32;
    fn MI_JPD_DirectBufDecode(dev: u32, chn: u32, input: *mut DirectIn, output: *mut DirectOut) -> i32;
    fn microcam_jpeg_from_yuyv(yuyv: *const u8, w: u32, h: u32, stride: u32, quality: i32, out: *mut *mut u8, len: *mut libc::c_ulong) -> i32;
    fn microcam_jpeg_free(p: *mut u8);
}

/// The engine's channel 0, one decode at a time.
static ENGINE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A JPEG file in the media heap, padded to 16 bytes.
pub struct Input {
    mem: Mma,
}

impl Input {
    pub fn read(jpeg: &[u8]) -> std::result::Result<Input, String> {
        let len = (jpeg.len() as u32).div_ceil(16) * 16;
        let mut mem = Mma::new(len, Use::Menu).map_err(|e| e.to_string())?;
        let b = mem.bytes();
        b[..jpeg.len()].copy_from_slice(jpeg);
        b[jpeg.len()..].fill(0);
        mem.sync().map_err(|e| e.to_string())?;
        Ok(Input { mem })
    }

    fn direct(&self) -> DirectIn {
        DirectIn { va: self.mem.read().as_ptr(), phy: self.mem.phy, len: self.mem.len() }
    }
}

/// The engine's picture: YUYV rows of `buf.w × 2` bytes, the picture `size` of them (the rest MCU padding).
pub struct Yuyv {
    mem: Mma,
    pub buf: Size,
    pub size: Size,
}

impl Yuyv {
    /// The picture as a source for SCL dev 3.
    pub(crate) fn direct(&self) -> ffi::MI_SCL_DirectBuf_t {
        ffi::MI_SCL_DirectBuf_t {
            ePixelFormat: PIXEL_YUYV,
            u32Width: self.buf.w,
            u32Height: self.buf.h,
            phyAddr: [self.mem.phy, 0],
            u32Stride: [self.buf.w * 2, 0],
            u32BuffSize: self.mem.len(),
        }
    }

    /// The picture brought to `size` (even sides) by SCL dev 3 for `user`, as a new YUYV buffer out of microcamd's share
    /// (a shrink of less than 1.2 aliases: the caller picks the decode's scale).
    pub fn scaled(&self, user: crate::scl::User, size: Size) -> std::result::Result<Yuyv, String> {
        let buf = Size::new((size.w + 15) & !15, size.h);
        let mem = Mma::new(buf.w * 2 * buf.h, Use::Menu).map_err(|e| e.to_string())?;
        // nothing of the CPU's may be written back over what the scaler writes
        mem.sync().map_err(|e| e.to_string())?;
        let out = Yuyv { mem, buf, size };
        let mut to = out.direct();
        (to.u32Width, to.u32Height) = (size.w, size.h);
        let all = microcam_core::Rect { x: 0, y: 0, w: self.size.w, h: self.size.h };
        crate::scl::stretch(user, &self.direct(), all, &to).map_err(|e| e.to_string())?;
        Ok(out)
    }

    /// The picture as a baseline 4:2:0 JPEG at `quality`, encoded by the CPU.
    pub fn jpeg(&self, quality: i32) -> std::result::Result<Vec<u8>, String> {
        // what the CPU may hold of these lines is older than what the engine wrote
        self.mem.sync().map_err(|e| e.to_string())?;
        let (mut out, mut len) = (std::ptr::null_mut(), 0);
        // SAFETY: the shim reads `size.h` rows of `size.w × 2` bytes `buf.w × 2` apart, inside the mapping; it allocates
        // `out`, freed below.
        let r = unsafe { microcam_jpeg_from_yuyv(self.mem.read().as_ptr(), self.size.w, self.size.h, self.buf.w * 2, quality, &mut out, &mut len) };
        if r != 0 || out.is_null() {
            return Err("JPEG encode failed".into());
        }
        // SAFETY: `len` bytes the shim allocated at `out`, copied, then freed once.
        let v = unsafe { std::slice::from_raw_parts(out, len as usize) }.to_vec();
        unsafe { microcam_jpeg_free(out) };
        Ok(v)
    }
}

/// The YUYV buffer a decode of a `full`-size picture at 1/`ratio` needs, in bytes (`ratio` 1, 2, 4 or 8): for the
/// price before the decode. Rounded up to the largest MCU (32 × 16).
pub fn bytes(full: Size, ratio: u32) -> u64 {
    let w = full.w.div_ceil(32) * 32 / ratio;
    let h = full.h.div_ceil(16) * 16 / ratio;
    w as u64 * h as u64 * 2
}

/// `input` (a `full`-size picture) decoded at 1/`ratio` (1, 2, 4 or 8) into a new YUYV buffer out of microcamd's share.
pub fn decode(input: &Input, full: Size, ratio: u32) -> std::result::Result<Yuyv, String> {
    // the module's scale rule (mhal-jpd.md §6.5): ratio 2 when a side is over the maximum, 4 over twice, 8 over four times
    let side = full.w.max(full.h);
    let max = if ratio <= 1 { WHOLE } else { side.div_ceil(ratio) };
    let mut a = ChnAttr { stream_buf_size: RING, codec: 0, max_w: max, max_h: max };
    let _one = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: plain structs we own; the module copies them.
    check("MI_JPD_CreateChn", unsafe { MI_JPD_CreateChn(DEV, CHN, &mut a) }).map_err(|e| e.to_string())?;
    let r = (|| {
        check("MI_JPD_StartChn", unsafe { MI_JPD_StartChn(DEV, CHN) }).map_err(|e| e.to_string())?;
        let mut info = StreamInfo::default();
        let mut i = input.direct();
        // SAFETY: `i` names memory `input` owns for this call; `info` is ours. A refusal (a progressive JPEG, say) is
        // the caller's fallback, not a warning.
        let q = unsafe { MI_JPD_QueryStreamInfo(DEV, CHN, &mut i, &mut info) };
        if q != 0 {
            return Err(format!("the hardware decoder does not read this JPEG ({q:#x})"));
        }
        if info.buf_w == 0 || info.buf_h == 0 || info.w != full.w || info.h != full.h {
            // a buffer of 0 bytes would be a kernel BUG in mi_sys's allocator
            return Err(format!("the hardware decoder reads {}x{} into {}x{}", info.w, info.h, info.buf_w, info.buf_h));
        }
        let buf = Size::new(info.buf_w, info.buf_h);
        let mem = Mma::new(buf.w * 2 * buf.h, Use::Menu).map_err(|e| e.to_string())?;
        let mut o = DirectOut { format: 0, w: buf.w, h: buf.h, stride: [buf.w * 2, 0], phy: [mem.phy, 0] };
        let mut i = input.direct();
        // SAFETY: the engine writes `buf.h` rows of `buf.w × 2` bytes at `mem`, which holds exactly that.
        check("MI_JPD_DirectBufDecode", unsafe { MI_JPD_DirectBufDecode(DEV, CHN, &mut i, &mut o) })
            .map_err(|e| format!("the hardware decoder: {e}"))?;
        let size = Size::new((full.w / ratio).min(buf.w) & !1, (full.h / ratio).min(buf.h) & !1);
        Ok(Yuyv { mem, buf, size })
    })();
    // SAFETY: the channel made above; no decode is in flight once the calls returned.
    unsafe {
        MI_JPD_StopChn(DEV, CHN);
        MI_JPD_DestroyChn(DEV, CHN);
    }
    r
}
