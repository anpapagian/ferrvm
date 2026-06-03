use ferrvm::printcrln;
use std::ptr::{self, NonNull};

/// Guest physical memory backed by an anonymous mmap.
pub struct GuestMemory {
    /// Pointer to the allocated memory region
    ptr: NonNull<u8>,
    /// Size of the memory region in bytes
    size: usize,
}

impl GuestMemory {
    pub fn allocate(size: usize) -> Result<Self, String> {
        if size == 0 {
            return Err("Memory size must be > 0".to_string());
        }

        // Align size to page boundary
        let page_size = 4096;
        let aligned_size = size.div_ceil(page_size) * page_size;

        printcrln!(
            "[memory] Allocating {size} bytes of guest RAM (aligned to {aligned_size} bytes)"
        );

        // SAFETY: anonymous mapping with a null hint and no fd; result is checked below.
        let ptr = unsafe {
            libc::mmap(
                ptr::null_mut(),
                aligned_size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_ANONYMOUS | libc::MAP_PRIVATE | libc::MAP_NORESERVE,
                -1,
                0,
            )
        };

        if ptr == libc::MAP_FAILED {
            return Err(format!(
                "Failed to allocate {aligned_size} bytes of guest memory"
            ));
        }

        printcrln!("[memory] Allocated {aligned_size} bytes of guest RAM at {ptr:p}");

        let ptr = NonNull::new(ptr.cast::<u8>())
            .ok_or_else(|| "mmap returned a null pointer for guest memory".to_string())?;

        Ok(Self {
            ptr,
            size: aligned_size,
        })
    }

    fn checked_hva(&self, gpa: u64, len: usize, op: &str) -> Result<*mut u8, String> {
        let start = usize::try_from(gpa)
            .map_err(|_| format!("{op} at 0x{gpa:X} cannot be represented on this host"))?;
        let end = start.checked_add(len).ok_or_else(|| {
            format!("{op} at 0x{gpa:X} (len {len}) overflows guest address space")
        })?;

        if end > self.size {
            return Err(format!(
                "{} at 0x{:X} (len {}) exceeds guest memory size (0x{:X})",
                op, gpa, len, self.size
            ));
        }

        // SAFETY: start lies within the allocation, checked above.
        Ok(unsafe { self.ptr.as_ptr().add(start) })
    }

    #[allow(dead_code)]
    pub fn zero_at(&self, gpa: u64, len: usize) -> Result<(), String> {
        let hva = self.checked_hva(gpa, len, "Zero")?;

        // SAFETY: hva is valid for len bytes, validated by checked_hva.
        unsafe {
            ptr::write_bytes(hva, 0, len);
        }

        Ok(())
    }

    pub fn read_at(&self, gpa: u64, len: usize) -> Result<Vec<u8>, String> {
        let hva = self.checked_hva(gpa, len, "Read")?;

        let mut buf = vec![0u8; len];

        // SAFETY: hva is valid for len bytes; buf is freshly allocated with len bytes.
        unsafe {
            ptr::copy_nonoverlapping(hva, buf.as_mut_ptr(), len);
        }

        Ok(buf)
    }

    pub fn write_at(&self, gpa: u64, data: &[u8]) -> Result<(), String> {
        let hva = self.checked_hva(gpa, data.len(), "Write")?;

        // SAFETY: hva is valid for data.len() bytes, validated by checked_hva.
        unsafe {
            ptr::copy_nonoverlapping(data.as_ptr(), hva, data.len());
        }

        Ok(())
    }

    pub unsafe fn write_obj<T: Sized>(&self, gpa: u64, obj: &T) -> Result<(), String> {
        // SAFETY: obj is a valid T, so its bytes are readable for size_of::<T>().
        let bytes = unsafe {
            std::slice::from_raw_parts(
                std::ptr::from_ref::<T>(obj).cast::<u8>(),
                std::mem::size_of::<T>(),
            )
        };
        self.write_at(gpa, bytes)
    }

    pub fn host_addr(&self) -> u64 {
        self.ptr.as_ptr() as u64
    }

    pub const fn size(&self) -> u64 {
        self.size as u64
    }
}

impl Drop for GuestMemory {
    fn drop(&mut self) {
        // SAFETY: ptr and size come from the mmap in allocate().
        unsafe {
            libc::munmap(self.ptr.as_ptr().cast::<libc::c_void>(), self.size);
        }
    }
}

// SAFETY: GuestMemory owns its mmap region; access is externally synchronized.
unsafe impl Send for GuestMemory {}
// SAFETY: GuestMemory owns its mmap region; access is externally synchronized.
unsafe impl Sync for GuestMemory {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allocate() {
        let size = 4096;
        let mem = GuestMemory::allocate(size).unwrap();
        assert_eq!(mem.size(), 4096);
        assert_ne!(mem.host_addr(), 0);
    }

    #[test]
    fn test_allocate_alignment() {
        let size = 100;
        let mem = GuestMemory::allocate(size).unwrap();
        assert_eq!(mem.size(), 4096); // Should be aligned to page size
    }

    #[test]
    fn test_allocate_zero() {
        let res = GuestMemory::allocate(0);
        assert!(res.is_err());
    }

    #[test]
    fn test_read_write() {
        let mem = GuestMemory::allocate(4096).unwrap();
        let data = b"hello world";
        mem.write_at(0x100, data).unwrap();
        let read = mem.read_at(0x100, data.len()).unwrap();
        assert_eq!(read, data);
    }

    #[test]
    fn test_zero_at() {
        let mem = GuestMemory::allocate(4096).unwrap();
        let data = b"hello world";
        mem.write_at(0x100, data).unwrap();
        mem.zero_at(0x100, data.len()).unwrap();
        let read = mem.read_at(0x100, data.len()).unwrap();
        assert_eq!(read, vec![0u8; data.len()]);
    }

    #[test]
    fn test_write_obj() {
        #[repr(C)]
        #[derive(Debug, PartialEq, Clone, Copy)]
        struct Foo {
            a: u32,
            b: u64,
        }

        let mem = GuestMemory::allocate(4096).unwrap();
        let obj = Foo {
            a: 0x1234_5678,
            b: 0xDEAD_BEEF_CAFE_BABE,
        };
        // SAFETY: Foo is a plain Copy struct with no invalid bit patterns.
        unsafe {
            mem.write_obj(0x200, &obj).unwrap();
        }

        let read = mem.read_at(0x200, std::mem::size_of::<Foo>()).unwrap();
        // SAFETY: read holds size_of::<Foo>() bytes written from a valid Foo.
        let read_obj: Foo = unsafe { std::ptr::read(read.as_ptr().cast()) };
        assert_eq!(read_obj, obj);
    }

    #[test]
    fn test_out_of_bounds() {
        let mem = GuestMemory::allocate(4096).unwrap();
        // Just past the end
        assert!(mem.read_at(4096, 1).is_err());
        // Starts in bounds, ends out of bounds
        assert!(mem.write_at(4090, b"1234567").is_err());
        // Large length that would overflow if not checked
        assert!(mem.zero_at(4000, usize::MAX - 4000 + 1).is_err());
        // Huge GPA
        assert!(mem.read_at(u64::MAX, 1).is_err());
    }
}
