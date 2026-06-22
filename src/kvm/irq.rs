use std::os::unix::io::RawFd;

use super::api::{KVM_IRQ_LINE, KVM_SIGNAL_MSI, KvmIrqLevel, KvmMsi};
use super::ioctl::ioctl_with_ref;
use crate::serial::{IrqSink, MsiSink};

pub struct KvmIrqSink {
    vm_fd: RawFd,
    gsi: u32,
}

impl KvmIrqSink {
    pub const fn new(vm_fd: RawFd, gsi: u32) -> Self {
        Self { vm_fd, gsi }
    }
}

impl IrqSink for KvmIrqSink {
    fn set_level(&self, asserted: bool) {
        let req = KvmIrqLevel {
            irq: self.gsi,
            level: u32::from(asserted),
        };
        // Safety: vm_fd must be valid.
        let _ = unsafe { ioctl_with_ref(self.vm_fd, KVM_IRQ_LINE, &req) };
    }
}

/// Injects MSI/MSI-X messages via `KVM_SIGNAL_MSI`. The in-kernel local APIC
/// decodes the address/data, so this works for any destination CPU and vector.
pub struct KvmMsiSink {
    vm_fd: RawFd,
}

impl KvmMsiSink {
    pub const fn new(vm_fd: RawFd) -> Self {
        Self { vm_fd }
    }
}

impl MsiSink for KvmMsiSink {
    fn signal(&self, address: u64, data: u32) {
        let msi = KvmMsi {
            address_lo: address as u32,
            address_hi: (address >> 32) as u32,
            data,
            flags: 0,
            devid: 0,
            pad: [0; 12],
        };
        // SAFETY: vm_fd is a valid VM handle and msi is a correctly sized KvmMsi.
        let _ = unsafe { ioctl_with_ref(self.vm_fd, KVM_SIGNAL_MSI, &msi) };
    }
}
