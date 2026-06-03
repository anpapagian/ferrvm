use std::io::{Error, Result};
use std::os::unix::io::RawFd;

pub unsafe fn ioctl_with_val(fd: RawFd, request: i32, arg: libc::c_ulong) -> Result<i32> {
    // SAFETY: caller guarantees fd is a valid vmm handle and request matches arg.
    let ret = unsafe { libc::ioctl(fd, request, arg) };
    if ret < 0 {
        Err(Error::last_os_error())
    } else {
        Ok(ret)
    }
}

pub unsafe fn ioctl_with_ref<T>(fd: RawFd, request: i32, arg: &T) -> Result<i32> {
    // SAFETY: caller guarantees fd is a valid vmm handle and arg is the correctly sized T for request.
    let ret = unsafe { libc::ioctl(fd, request, std::ptr::from_ref::<T>(arg)) };
    if ret < 0 {
        Err(Error::last_os_error())
    } else {
        Ok(ret)
    }
}

pub unsafe fn ioctl_with_mut_ref<T>(fd: RawFd, request: i32, arg: &mut T) -> Result<i32> {
    // SAFETY: caller guarantees fd is a valid vmm handle and arg is the correctly sized T for request.
    let ret = unsafe { libc::ioctl(fd, request, std::ptr::from_mut::<T>(arg)) };
    if ret < 0 {
        Err(Error::last_os_error())
    } else {
        Ok(ret)
    }
}
