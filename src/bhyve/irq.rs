use std::os::unix::io::RawFd;

use super::api::{VM_ISA_ASSERT_IRQ, VM_ISA_DEASSERT_IRQ, VmIsaIrq};
use super::ioctl::ioctl_with_ref;
use crate::serial::IrqSink;

pub struct BhyveIrqSink {
    vm_fd: RawFd,
    gsi: u32,
}

impl BhyveIrqSink {
    pub const fn new(vm_fd: RawFd, gsi: u32) -> Self {
        Self { vm_fd, gsi }
    }
}

impl IrqSink for BhyveIrqSink {
    fn set_level(&self, asserted: bool) {
        let req = VmIsaIrq {
            atpic_irq: self.gsi as i32,
            ioapic_irq: self.gsi as i32,
        };
        let request = if asserted {
            VM_ISA_ASSERT_IRQ
        } else {
            VM_ISA_DEASSERT_IRQ
        };
        // SAFETY: vm_fd is a valid bhyve vm handle and req is the correct arg for an isa irq ioctl.
        let _ = unsafe { ioctl_with_ref(self.vm_fd, request as i32, &req) };
    }
}
