use std::io::{Error, Result};
use std::os::unix::io::RawFd;

const _IOC_SIZEBITS: u32 = 14;
const _IOC_DIRBITS: u32 = 2;
const _IOC_NRBITS: u32 = 8;
const _IOC_TYPEBITS: u32 = 8;

const _IOC_NRMASK: u32 = (1 << _IOC_NRBITS) - 1;
const _IOC_TYPEMASK: u32 = (1 << _IOC_TYPEBITS) - 1;
const _IOC_SIZEMASK: u32 = (1 << _IOC_SIZEBITS) - 1;
const _IOC_DIRMASK: u32 = (1 << _IOC_DIRBITS) - 1;

const _IOC_NRSHIFT: u32 = 0;
const _IOC_TYPESHIFT: u32 = _IOC_NRSHIFT + _IOC_NRBITS;
const _IOC_SIZESHIFT: u32 = _IOC_TYPESHIFT + _IOC_TYPEBITS;
const _IOC_DIRSHIFT: u32 = _IOC_SIZESHIFT + _IOC_SIZEBITS;

// Linux ioctl direction bits
const _IOC_NONE: u32 = 0;
const _IOC_WRITE: u32 = 1;
const _IOC_READ: u32 = 2;

#[inline]
const fn ioc(dir: u32, type_code: u32, nr: u32, size: u32) -> u64 {
    ((dir << _IOC_DIRSHIFT)
        | (type_code << _IOC_TYPESHIFT)
        | (nr << _IOC_NRSHIFT)
        | (size << _IOC_SIZESHIFT)) as u64
}

#[inline]
pub const fn io(type_code: u32, nr: u32) -> u64 {
    ioc(_IOC_NONE, type_code, nr, 0)
}

#[inline]
pub const fn ior<T>(type_code: u32, nr: u32) -> u64 {
    ioc(_IOC_READ, type_code, nr, std::mem::size_of::<T>() as u32)
}

#[inline]
pub const fn iow<T>(type_code: u32, nr: u32) -> u64 {
    ioc(_IOC_WRITE, type_code, nr, std::mem::size_of::<T>() as u32)
}

#[inline]
pub const fn iowr<T>(type_code: u32, nr: u32) -> u64 {
    ioc(
        _IOC_READ | _IOC_WRITE,
        type_code,
        nr,
        std::mem::size_of::<T>() as u32,
    )
}

pub unsafe fn ioctl_with_val(fd: RawFd, request: u64, arg: libc::c_ulong) -> Result<i32> {
    // SAFETY: caller guarantees fd is a valid KVM handle and request takes a c_ulong arg.
    let ret = unsafe { libc::ioctl(fd, request, arg) };
    if ret < 0 {
        Err(Error::last_os_error())
    } else {
        Ok(ret)
    }
}

pub unsafe fn ioctl_with_ref<T>(fd: RawFd, request: u64, arg: &T) -> Result<i32> {
    // SAFETY: caller guarantees fd is valid and request matches the layout of T behind arg.
    let ret = unsafe { libc::ioctl(fd, request, std::ptr::from_ref::<T>(arg)) };
    if ret < 0 {
        Err(Error::last_os_error())
    } else {
        Ok(ret)
    }
}

pub unsafe fn ioctl_with_mut_ref<T>(fd: RawFd, request: u64, arg: &mut T) -> Result<i32> {
    // SAFETY: caller guarantees fd is valid and request matches the layout of T behind arg.
    let ret = unsafe { libc::ioctl(fd, request, std::ptr::from_mut::<T>(arg)) };
    if ret < 0 {
        Err(Error::last_os_error())
    } else {
        Ok(ret)
    }
}
