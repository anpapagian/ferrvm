//! Generic I/O bus: maps (base, len) -> device and dispatches reads/writes.
//!
//! Used for both PIO (16-bit port address space) and MMIO (64-bit physical
//! address space).
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// Error type for bus registration. Dispatch itself cannot fail from the
/// caller's perspective: unmapped accesses are handled silently (reads return
/// 0xff, writes are dropped), matching real hardware's behavior and what guest
/// probe code expects.
#[derive(Debug, thiserror::Error)]
pub enum BusError {
    #[error("zero-length range at base {0:#x}")]
    ZeroLength(u64),
    #[error("range [{base:#x}, {end:#x}) overflows u64")]
    Overflow { base: u64, end: u64 },
    #[error("range [{base:#x}, +{len:#x}) overlaps existing device")]
    Overlap { base: u64, len: u64 },
}

/// Anything that lives on a bus. Offsets are relative to the device's base;
/// i.e. a serial registered at 0x3f8 sees offset 0..8 for its own registers.
///
/// Implementors must tolerate any access width the guest legally produces for
/// this device class (typically 1/2/4 bytes for PIO, 1/2/4/8 for MMIO).
pub trait BusDevice: Send {
    /// Read from `offset`, filling `data` entirely. `data.len()` is the
    /// access width in bytes.
    fn read(&mut self, offset: u64, data: &mut [u8]);

    /// Write `data` to `offset`. `data.len()` is the access width in bytes.
    fn write(&mut self, offset: u64, data: &[u8]);
}

/// A range `[base, base+len)` on the bus.
#[derive(Copy, Clone, Debug)]
pub struct BusRange {
    pub base: u64,
    pub len: u64,
}

impl BusRange {
    pub fn new(base: u64, len: u64) -> Result<Self, BusError> {
        if len == 0 {
            return Err(BusError::ZeroLength(base));
        }
        let end = base.checked_add(len).ok_or(BusError::Overflow {
            base,
            end: u64::MAX,
        })?;
        let _ = end; // end is only used for the overflow check
        Ok(Self { base, len })
    }

    /// One-byte range used as a lookup key for a point address.
    #[inline]
    const fn point(addr: u64) -> Self {
        // Safe for lookups: `end()` saturates, so even `u64::MAX` will not
        // overflow during comparisons.
        Self { base: addr, len: 1 }
    }
    #[inline]
    const fn end(&self) -> u64 {
        // Registered ranges satisfy the construction invariant, so this is
        // equivalent to `base + len` for them. Use saturating addition so the
        // lookup-only range produced by `point(u64::MAX)` cannot panic.
        self.base.saturating_add(self.len)
    }

    #[inline]
    fn contains_range(&self, addr: u64, len: u64) -> bool {
        // Does [addr, addr+len) lie fully within self?
        addr.checked_add(len)
            .is_some_and(|req_end| addr >= self.base && req_end <= self.end())
    }

    #[inline]
    const fn overlaps(&self, other: &Self) -> bool {
        self.base < other.end() && other.base < self.end()
    }
}

impl PartialEq for BusRange {
    fn eq(&self, other: &Self) -> bool {
        self.overlaps(other)
    }
}

impl Eq for BusRange {}

impl PartialOrd for BusRange {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BusRange {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering::{Equal, Greater, Less};
        if self.overlaps(other) {
            Equal
        } else if self.end() <= other.base {
            Less
        } else {
            Greater
        }
    }
}

/// A bus: range -> device map with O(log n) dispatch.
#[derive(Clone, Default)]
pub struct Bus {
    inner: Arc<Mutex<BusInner>>,
}

#[derive(Default)]
struct BusInner {
    devices: BTreeMap<BusRange, Arc<Mutex<dyn BusDevice>>>,
}

impl Bus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `device` at `[base, base+len)`. Returns `BusError::Overlap` if
    /// the range collides with any existing device.
    pub fn register(
        &self,
        base: u64,
        len: u64,
        device: Arc<Mutex<dyn BusDevice>>,
    ) -> Result<(), BusError> {
        let range = BusRange::new(base, len)?;
        let mut inner = self.inner.lock().expect("bus map poisoned");
        if inner.devices.contains_key(&range) {
            return Err(BusError::Overlap { base, len });
        }
        inner.devices.insert(range, device);
        drop(inner);
        Ok(())
    }

    /// Dispatch a read. Returns `true` if a device claimed the access.
    /// On unclaimed reads, `data` is filled with 0xff (hardware pull-up
    /// convention) and `false` is returned — caller may log but must not
    /// fail the vcpu.
    pub fn read(&self, addr: u64, data: &mut [u8]) -> bool {
        if data.is_empty() {
            return true;
        }
        let Some((range, dev)) = self.lookup(addr) else {
            data.fill(0xff);
            return false;
        };
        if !range.contains_range(addr, data.len() as u64) {
            // Access straddles the device's end. Real hardware would split
            // this across two bus cycles; KVM will never hand us such an
            // access for PIO (widths are 1/2/4 and always register-aligned
            // within a device) but MMIO callers should be aware.
            data.fill(0xff);
            return false;
        }
        let offset = addr - range.base;
        dev.lock().expect("device poisoned").read(offset, data);
        true
    }

    /// Dispatch a write. Returns `true` if a device claimed the access.
    /// Unclaimed writes are dropped silently.
    pub fn write(&self, addr: u64, data: &[u8]) -> bool {
        if data.is_empty() {
            return true;
        }
        let Some((range, dev)) = self.lookup(addr) else {
            return false;
        };
        if !range.contains_range(addr, data.len() as u64) {
            return false;
        }
        let offset = addr - range.base;
        dev.lock().expect("device poisoned").write(offset, data);
        true
    }

    fn lookup(&self, addr: u64) -> Option<(BusRange, Arc<Mutex<dyn BusDevice>>)> {
        let inner = self.inner.lock().expect("bus map poisoned");
        inner
            .devices
            .get_key_value(&BusRange::point(addr))
            .map(|(r, d)| (*r, Arc::clone(d)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockDevice {
        last_offset: u64,
        last_data: Vec<u8>,
    }

    impl MockDevice {
        fn new() -> Self {
            Self {
                last_offset: 0,
                last_data: Vec::new(),
            }
        }
    }

    impl BusDevice for MockDevice {
        fn read(&mut self, offset: u64, data: &mut [u8]) {
            self.last_offset = offset;
            for (i, b) in data.iter_mut().enumerate() {
                *b = (offset as u8).wrapping_add(i as u8);
            }
        }

        fn write(&mut self, offset: u64, data: &[u8]) {
            self.last_offset = offset;
            self.last_data = data.to_vec();
        }
    }

    #[test]
    fn test_bus_range_overlap() {
        let r1 = BusRange::new(0x100, 0x10).unwrap();
        let r2 = BusRange::new(0x108, 0x10).unwrap();
        let r3 = BusRange::new(0x110, 0x10).unwrap();

        assert!(r1.overlaps(&r2));
        assert!(r2.overlaps(&r1));
        assert!(!r1.overlaps(&r3));
        assert!(r2.overlaps(&r3));
    }

    #[test]
    fn test_bus_registration() {
        let bus = Bus::new();
        let dev = Arc::new(Mutex::new(MockDevice::new()));

        bus.register(0x1000, 0x10, dev.clone()).unwrap();
        // Just before, no overlap
        bus.register(0x900, 0x100, dev.clone()).unwrap();
        // Overlap at start of first device
        assert!(matches!(
            bus.register(0x900, 0x701, dev.clone()),
            Err(BusError::Overlap { .. })
        ));
        // Overlap at end of first device
        assert!(matches!(
            bus.register(0x1008, 0x10, dev.clone()),
            Err(BusError::Overlap { .. })
        ));
        // Full overlap
        assert!(matches!(
            bus.register(0x1002, 0x4, dev),
            Err(BusError::Overlap { .. })
        ));
    }

    #[test]
    fn test_bus_dispatch() {
        let bus = Bus::new();
        let dev = Arc::new(Mutex::new(MockDevice::new()));
        bus.register(0x1000, 0x10, dev.clone()).unwrap();

        // Successful read
        let mut data = [0u8; 4];
        assert!(bus.read(0x1002, &mut data));
        assert_eq!(data, [2, 3, 4, 5]);
        assert_eq!(dev.lock().unwrap().last_offset, 2);

        // Successful write
        bus.write(0x1004, &[0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(dev.lock().unwrap().last_offset, 4);
        assert_eq!(dev.lock().unwrap().last_data, vec![0xde, 0xad, 0xbe, 0xef]);

        // Unmapped read
        let mut data = [0u8; 2];
        assert!(!bus.read(0x2000, &mut data));
        assert_eq!(data, [0xff, 0xff]);

        // Straddled read
        let mut data = [0u8; 4];
        assert!(!bus.read(0x100e, &mut data));
        assert_eq!(data, [0xff, 0xff, 0xff, 0xff]);
    }
}
