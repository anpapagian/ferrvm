use crate::serial::IrqSink;

use super::api;

pub struct HvfIrqSink {
    intid: u32,
}

impl HvfIrqSink {
    pub const fn new(intid: u32) -> Self {
        Self { intid }
    }
}

impl IrqSink for HvfIrqSink {
    fn set_level(&self, asserted: bool) {
        // SAFETY: The VM owns a native GIC for the lifetime of every IRQ sink.
        let rc = unsafe { api::hv_gic_set_spi(self.intid, asserted) };
        if rc != api::HV_SUCCESS {
            eprintln!(
                "[hvf] failed to set interrupt {} to {}: {rc:#x}",
                self.intid, asserted
            );
        }
    }
}
