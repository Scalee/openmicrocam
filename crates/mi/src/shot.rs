//! Frames of the media port taken by hand: for a photo ([`crate::compose::Composer::shoot`]) one programmed after the
//! `ui` handed over the canvases of its menu (microcam_rgn's frame count, [`crate::rgn::frames`]), so it holds exactly the
//! scene it was asked with; a camera picture (the menu's `camera` resource, [`grab`]) is the newest frame copied by the
//! memory-to-memory scaler into a picture of its own.

use microcam_core::reconcile::ChnPort;
use microcam_core::Size;
use mi_sys as ffi;

use crate::picture::Frame;
use crate::sys::chn_port;
use crate::check;

type Taken = std::result::Result<(ffi::MI_SYS_BUF_HANDLE, ffi::MI_SYS_BufInfo_t), String>;

/// The newest frame of `src` (`size`, fed to nothing) copied 1:1 by SCL dev 3 into a new picture out of microcamd's share
/// (`size`'s width a multiple of 16, the scaler's rule for its source; even height). Waits up to `timeout_ms`.
pub fn grab(src: ChnPort, size: Size, timeout_ms: i32) -> std::result::Result<Frame, String> {
    let pic = Frame::empty(size)?;
    let (h, info) = take(src, size, None, timeout_ms)?;
    // SAFETY: the frame view of a frame buffer the library handed out.
    let d = unsafe { &info.__bindgen_anon_1.stFrameData };
    let from = ffi::MI_SCL_DirectBuf_t {
        ePixelFormat: crate::PIXEL_NV12,
        u32Width: size.w,
        u32Height: size.h,
        phyAddr: [d.phyAddr[0], d.phyAddr[1]],
        u32Stride: [d.u32Stride[0], d.u32Stride[1]],
        u32BuffSize: d.u32BufSize,
    };
    let r = crate::scl::stretch(crate::scl::User::Pipeline, &from, microcam_core::Rect::full(size), &pic.direct());
    unsafe { ffi::MI_SYS_ChnOutputPortPutBufPa(h) };
    r.map_err(|e| e.to_string())?;
    // what the CPU may hold of these lines is older than what the scaler wrote
    pic.sync().map_err(|e| e.to_string())?;
    Ok(pic)
}

/// The frame to hand on. With `from`: once microcam_rgn counts the port's frame `from` done, every frame queued then goes
/// back and the next one to come is taken — it was programmed at `from` or later, whichever way the port's queue and
/// the done count are ordered. Without: the newest queued (older ones go back), or the next within `timeout_ms`.
pub(crate) fn take(src: ChnPort, size: Size, from: Option<u64>, timeout_ms: i32) -> Taken {
    let mut port = chn_port(src);
    let mut fd = -1;
    check("MI_SYS_GetFd", unsafe { ffi::MI_SYS_GetFd(&mut port, &mut fd) }).map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms.max(0) as u64);
    // a frame `from` or later may be taken: from the start without `from`, once its frame is done with it
    let mut armed = from.is_none();
    let mut got: Option<(ffi::MI_SYS_BUF_HANDLE, ffi::MI_SYS_BufInfo_t)> = None;
    let r = loop {
        if !armed {
            match crate::rgn::frames(src) {
                Ok((_, done)) => armed = done >= from.unwrap_or(0),
                Err(e) => break Err(e.to_string()),
            }
            // what the port holds now may be older than `from`: it goes back either way
            while let Some((h, _)) = get(&mut port) {
                unsafe { ffi::MI_SYS_ChnOutputPortPutBufPa(h) };
            }
        } else {
            while let Some(f) = get(&mut port) {
                if let Some((old, _)) = got.replace(f) {
                    unsafe { ffi::MI_SYS_ChnOutputPortPutBufPa(old) };
                }
                // with `from` the first one is the frame: newer ones stay queued
                if from.is_some() {
                    break;
                }
            }
        }
        if let Some((h, info)) = got {
            // SAFETY: the frame view of a frame buffer the library handed out.
            let d = unsafe { &info.__bindgen_anon_1.stFrameData };
            let at = Size::new(d.u16Width as u32, d.u16Height as u32);
            if at == size {
                break Ok((h, info));
            }
            unsafe { ffi::MI_SYS_ChnOutputPortPutBufPa(h) };
            break Err(format!("the port hands on {at}, not {size}"));
        }
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            break Err(match from {
                Some(f) if !armed => format!("microcam_rgn did not count frame {f} done in {timeout_ms} ms"),
                _ => format!("no frame in {timeout_ms} ms"),
            });
        }
        let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        // SAFETY: one pollfd we own.
        unsafe { libc::poll(&mut pfd, 1, left.as_millis().min(50) as i32) };
    };
    unsafe { ffi::MI_SYS_CloseFd(fd) };
    r
}

/// The next frame the port has queued for us, if any.
fn get(port: &mut ffi::MI_SYS_ChnPort_t) -> Option<(ffi::MI_SYS_BUF_HANDLE, ffi::MI_SYS_BufInfo_t)> {
    // SAFETY: all-zero is a valid BufInfo (plain data).
    let mut info: ffi::MI_SYS_BufInfo_t = unsafe { std::mem::zeroed() };
    let mut h: ffi::MI_SYS_BUF_HANDLE = 0;
    (unsafe { ffi::MI_SYS_ChnOutputPortGetBufPa(port, &mut info, &mut h) } == 0).then_some((h, info))
}
