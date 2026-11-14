//! ioctl request numbers for the UVC gadget node, computed the way the kernel's `_IOC` macro does on ARM (generic
//! layout: nr 8 bits, type 8, size 14, dir 2) from the sizes of the bound UAPI structs.

use std::mem::size_of;

use mi_sys::uapi as u;

const NONE: u32 = 0;
const WRITE: u32 = 1;
const READ: u32 = 2;

const fn ioc(dir: u32, ty: u8, nr: u8, size: usize) -> u32 {
    (dir << 30) | ((size as u32) << 16) | ((ty as u32) << 8) | nr as u32
}

pub const VIDIOC_QUERYCAP: u32 = ioc(READ, b'V', 0, size_of::<u::v4l2_capability>());
pub const VIDIOC_S_FMT: u32 = ioc(READ | WRITE, b'V', 5, size_of::<u::v4l2_format>());
pub const VIDIOC_REQBUFS: u32 = ioc(READ | WRITE, b'V', 8, size_of::<u::v4l2_requestbuffers>());
pub const VIDIOC_QUERYBUF: u32 = ioc(READ | WRITE, b'V', 9, size_of::<u::v4l2_buffer>());
pub const VIDIOC_QBUF: u32 = ioc(READ | WRITE, b'V', 15, size_of::<u::v4l2_buffer>());
pub const VIDIOC_DQBUF: u32 = ioc(READ | WRITE, b'V', 17, size_of::<u::v4l2_buffer>());
pub const VIDIOC_STREAMON: u32 = ioc(WRITE, b'V', 18, size_of::<i32>());
pub const VIDIOC_STREAMOFF: u32 = ioc(WRITE, b'V', 19, size_of::<i32>());
pub const VIDIOC_DQEVENT: u32 = ioc(READ, b'V', 89, size_of::<u::v4l2_event>());
pub const VIDIOC_SUBSCRIBE_EVENT: u32 = ioc(WRITE, b'V', 90, size_of::<u::v4l2_event_subscription>());
pub const UVCIOC_SEND_RESPONSE: u32 = ioc(WRITE, b'U', 1, size_of::<u::uvc_request_data>());

#[allow(dead_code)]
const _: () = assert!(NONE == 0);

/// `ioctl` with an errno result.
pub fn call<T>(fd: i32, req: u32, arg: &mut T) -> std::io::Result<()> {
    // SAFETY: `arg` is a live, correctly typed struct for `req`; the kernel reads/writes within its size.
    let r = unsafe { libc::ioctl(fd, req as _, arg as *mut T) };
    if r < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn call_int(fd: i32, req: u32, mut v: i32) -> std::io::Result<()> {
    call(fd, req, &mut v)
}
