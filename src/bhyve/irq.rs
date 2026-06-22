use std::os::unix::io::RawFd;

use super::api::{VM_ISA_ASSERT_IRQ, VM_ISA_DEASSERT_IRQ, VM_LAPIC_MSI, VmIsaIrq, VmLapicMsi};
use super::ioctl::ioctl_with_ref;
use crate::serial::{IrqSink, MsiSink};

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

/// Injects MSI/MSI-X messages via `VM_LAPIC_MSI`. The in-kernel local APIC
/// decodes the address/data, so this works for any destination CPU and vector.
pub struct BhyveMsiSink {
    vm_fd: RawFd,
}

impl BhyveMsiSink {
    pub const fn new(vm_fd: RawFd) -> Self {
        Self { vm_fd }
    }
}

impl MsiSink for BhyveMsiSink {
    fn signal(&self, address: u64, data: u32) {
        let req = VmLapicMsi {
            msg: u64::from(data),
            addr: address,
        };
        // SAFETY: vm_fd is a valid bhyve vm handle and req is the correct arg for VM_LAPIC_MSI.
        let _ = unsafe { ioctl_with_ref(self.vm_fd, VM_LAPIC_MSI as i32, &req) };
    }
}
