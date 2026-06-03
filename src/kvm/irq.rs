use std::os::unix::io::RawFd;

use super::api::{KVM_IRQ_LINE, KvmIrqLevel};
use super::ioctl::ioctl_with_ref;
use crate::serial::IrqSink;

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
