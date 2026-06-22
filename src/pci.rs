//! PCI host bridge and legacy type-1 configuration mechanism.
//!
//! Only a single bus with no bridges is implemented.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::bus::BusDevice;

/// A device that owns one 256-byte PCI configuration space.
pub trait PciDevice: Send {
    /// Read `data.len()` bytes of configuration space starting at `offset`.
    fn config_read(&self, offset: u64, data: &mut [u8]);

    /// Write `data` into configuration space starting at `offset`.
    fn config_write(&mut self, offset: u64, data: &[u8]);
}

/// A plain 256-byte configuration space backing store.
pub struct ConfigSpace {
    regs: [u8; 256],
}

impl Default for ConfigSpace {
    fn default() -> Self {
        Self { regs: [0; 256] }
    }
}

impl ConfigSpace {
    /// Create an all-zero configuration space.
    pub const fn new() -> Self {
        Self { regs: [0; 256] }
    }

    /// Store a byte at `offset`.
    pub const fn set_u8(&mut self, offset: usize, value: u8) {
        self.regs[offset] = value;
    }

    /// Store a little-endian 16-bit value at `offset`.
    pub const fn set_u16(&mut self, offset: usize, value: u16) {
        self.regs[offset] = value as u8;
        self.regs[offset + 1] = (value >> 8) as u8;
    }

    /// Store a little-endian 32-bit value at `offset`.
    pub const fn set_u32(&mut self, offset: usize, value: u32) {
        self.regs[offset] = value as u8;
        self.regs[offset + 1] = (value >> 8) as u8;
        self.regs[offset + 2] = (value >> 16) as u8;
        self.regs[offset + 3] = (value >> 24) as u8;
    }

    /// Load a little-endian 16-bit value from `offset`.
    pub const fn get_u16(&self, offset: usize) -> u16 {
        (self.regs[offset] as u16) | ((self.regs[offset + 1] as u16) << 8)
    }

    /// Load a little-endian 32-bit value from `offset`.
    pub const fn get_u32(&self, offset: usize) -> u32 {
        (self.regs[offset] as u32)
            | ((self.regs[offset + 1] as u32) << 8)
            | ((self.regs[offset + 2] as u32) << 16)
            | ((self.regs[offset + 3] as u32) << 24)
    }
}

impl PciDevice for ConfigSpace {
    /// Bytes past the end of the 256-byte space read back as `0xff`.
    fn config_read(&self, offset: u64, data: &mut [u8]) {
        for (i, b) in data.iter_mut().enumerate() {
            let idx = offset as usize + i;
            *b = if idx < self.regs.len() {
                self.regs[idx]
            } else {
                0xff
            };
        }
    }

    fn config_write(&mut self, _offset: u64, _data: &[u8]) {}
}

/// Configuration space for an Intel 440FX (`8086:1237`) host bridge at slot
/// 00:00.0. It carries no BARs and is read-only; its sole purpose is to give
/// the guest a recognizable, well-formed device in slot 0 so PCI enumeration
/// proceeds.
pub const fn host_bridge() -> ConfigSpace {
    let mut cs = ConfigSpace::new();
    cs.set_u16(0x00, 0x8086); // Vendor ID: Intel
    cs.set_u16(0x02, 0x1237); // Device ID: 440FX
    cs.set_u8(0x08, 0x02); // Revision
    cs.set_u8(0x09, 0x00); // Prog IF
    cs.set_u8(0x0a, 0x00); // Subclass: Host bridge
    cs.set_u8(0x0b, 0x06); // Class: Bridge
    cs.set_u8(0x0e, 0x00); // Header type 0
    cs
}

/// PCI bus 0 and the type-1 configuration mechanism.
#[derive(Default)]
pub struct PciRootBus {
    /// Last value written to the `CONFIG_ADDRESS` port (`0xcf8`).
    config_address: u32,
    /// Devices keyed by [`slot_key`] (device and function on bus 0).
    devices: HashMap<u16, Arc<Mutex<dyn PciDevice>>>,
}

impl PciRootBus {
    /// Create an empty bus with no devices.
    pub fn new() -> Self {
        Self::default()
    }

    /// Place `device` in slot `dev`/`func` on bus 0.
    pub fn add_device(&mut self, dev: u8, func: u8, device: Arc<Mutex<dyn PciDevice>>) {
        self.devices.insert(slot_key(dev, func), device);
    }

    /// Decode the latched `CONFIG_ADDRESS` plus the byte offset within the
    /// `CONFIG_DATA` window into a `(slot key, register)` pair.
    ///
    /// Returns `None` when the enable bit (bit 31) is clear or the address
    /// targets a bus other than 0 — both decode to "no device".
    fn selected(&self, port_byte: u64) -> Option<(u16, u64)> {
        if self.config_address & 0x8000_0000 == 0 {
            return None;
        }
        let bus = (self.config_address >> 16) & 0xff;
        if bus != 0 {
            return None;
        }
        let dev = ((self.config_address >> 11) & 0x1f) as u8;
        let func = ((self.config_address >> 8) & 0x07) as u8;
        let reg = u64::from(self.config_address & 0xfc) + port_byte;
        Some((slot_key(dev, func), reg))
    }
}

const fn slot_key(dev: u8, func: u8) -> u16 {
    ((dev as u16) << 3) | (func as u16 & 0x07)
}

impl BusDevice for PciRootBus {
    /// Offsets below 4 read back the `CONFIG_ADDRESS` latch; the rest read the
    /// selected device's configuration register (or `0xff` if none).
    fn read(&mut self, offset: u64, data: &mut [u8]) {
        if offset < 4 {
            let bytes = self.config_address.to_le_bytes();
            for (i, b) in data.iter_mut().enumerate() {
                let idx = offset as usize + i;
                *b = bytes.get(idx).copied().unwrap_or(0);
            }
            return;
        }

        match self.selected(offset - 4) {
            Some((key, reg)) => match self.devices.get(&key) {
                Some(dev) => dev
                    .lock()
                    .expect("pci device poisoned")
                    .config_read(reg, data),
                None => data.fill(0xff),
            },
            None => data.fill(0xff),
        }
    }

    /// Offsets below 4 update the `CONFIG_ADDRESS` latch; the rest forward to
    /// the selected device's configuration register (a no-op if none).
    fn write(&mut self, offset: u64, data: &[u8]) {
        if offset < 4 {
            let mut bytes = self.config_address.to_le_bytes();
            for (i, &v) in data.iter().enumerate() {
                let idx = offset as usize + i;
                if idx < bytes.len() {
                    bytes[idx] = v;
                }
            }
            self.config_address = u32::from_le_bytes(bytes);
            return;
        }

        if let Some((key, reg)) = self.selected(offset - 4)
            && let Some(dev) = self.devices.get(&key)
        {
            dev.lock()
                .expect("pci device poisoned")
                .config_write(reg, data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> PciRootBus {
        let mut bus = PciRootBus::new();
        bus.add_device(0, 0, Arc::new(Mutex::new(host_bridge())));
        bus
    }

    /// Latch the address of `reg` on slot `dev` and read `CONFIG_DATA` as a dword.
    fn read_cfg(bus: &mut PciRootBus, dev: u8, reg: u32) -> u32 {
        let addr: u32 = 0x8000_0000 | (u32::from(dev) << 11) | (reg & 0xfc);
        bus.write(0, &addr.to_le_bytes());
        let mut data = [0u8; 4];
        bus.read(4, &mut data);
        u32::from_le_bytes(data)
    }

    #[test]
    fn config_address_reads_back() {
        let mut bus = setup();
        bus.write(0, &0x8000_0000u32.to_le_bytes());
        let mut data = [0u8; 4];
        bus.read(0, &mut data);
        assert_eq!(u32::from_le_bytes(data), 0x8000_0000);
    }

    #[test]
    fn reads_host_bridge_vendor_device() {
        let mut bus = setup();
        let id = read_cfg(&mut bus, 0, 0x00);
        assert_eq!(id & 0xffff, 0x8086); // Intel
        assert_eq!(id >> 16, 0x1237); // 440FX
    }

    #[test]
    fn host_bridge_class_is_bridge() {
        let mut bus = setup();
        let class = read_cfg(&mut bus, 0, 0x08);
        assert_eq!(class >> 24, 0x06); // base class: bridge
    }

    #[test]
    fn absent_slot_reads_all_ones() {
        let mut bus = setup();
        assert_eq!(read_cfg(&mut bus, 1, 0x00), 0xffff_ffff);
    }

    #[test]
    fn disabled_address_reads_all_ones() {
        let mut bus = setup();
        // No enable bit (bit 31) -> CONFIG_DATA decodes to nothing.
        bus.write(0, &0u32.to_le_bytes());
        let mut data = [0u8; 4];
        bus.read(4, &mut data);
        assert_eq!(data, [0xff; 4]);
    }
}
