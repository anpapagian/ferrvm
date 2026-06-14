use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::bus::BusDevice;

pub trait PciDevice: Send {
    fn config_read(&self, offset: u64, data: &mut [u8]);
    fn config_write(&mut self, offset: u64, data: &[u8]);
}

pub struct ConfigSpace {
    regs: [u8; 256],
}

impl Default for ConfigSpace {
    fn default() -> Self {
        Self { regs: [0; 256] }
    }
}

impl ConfigSpace {
    pub fn new() -> Self {
        Self::default()
    }

    pub const fn set_u8(&mut self, offset: usize, value: u8) {
        self.regs[offset] = value;
    }

    pub const fn set_u16(&mut self, offset: usize, value: u16) {
        self.regs[offset] = value as u8;
        self.regs[offset + 1] = (value >> 8) as u8;
    }
}

impl PciDevice for ConfigSpace {
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

pub fn host_bridge() -> ConfigSpace {
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

#[derive(Default)]
pub struct PciRootBus {
    config_address: u32,
    devices: HashMap<u16, Arc<Mutex<dyn PciDevice>>>,
}

impl PciRootBus {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_device(&mut self, dev: u8, func: u8, device: Arc<Mutex<dyn PciDevice>>) {
        self.devices.insert(slot_key(dev, func), device);
    }

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
