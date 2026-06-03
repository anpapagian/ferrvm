use std::sync::Arc;

pub use crate::serial::IrqSink;
use crate::stats::ExitStats;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub trait Hypervisor {
    fn create_vm<'a>(&'a self, name: &str) -> Result<Box<dyn Vm + 'a>>;
}

pub trait Vm {
    fn setup(&self) -> Result<()>;
    fn create_vcpu<'a>(&'a self, id: u32) -> Result<Box<dyn Vcpu + Send + 'a>>;
    fn register_memory_region(&self, gpa: u64, size: u64, hpa: u64) -> Result<()>;
    fn register_irq(&self, irq: u32) -> Result<Arc<dyn IrqSink>>;
}

pub trait Vcpu {
    fn setup(&self) -> Result<()>;
    fn step_run(
        &mut self,
        stats: &mut ExitStats,
        memory: &crate::memory::GuestMemory,
        pio_bus: &crate::bus::Bus,
        mmio_bus: &crate::bus::Bus,
        config: &crate::config::Config,
    ) -> core::result::Result<bool, std::io::Error>;
    fn set_regs(&self, regs: &VcpuRegs) -> Result<()>;
    fn get_regs(&self) -> Result<VcpuRegs>;
    fn set_sregs(&self, sregs: &VcpuSregs) -> Result<()>;
    fn get_sregs(&self) -> Result<VcpuSregs>;
}

#[derive(Debug, Default, Clone)]
pub struct VcpuRegs {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rsp: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
}

/// Segment Register (for GDT, LDT, TR, etc.)
#[derive(Clone, Debug, Default)]
pub struct Segment {
    pub base: u64,
    pub limit: u32,
    pub selector: u16,
    pub type_: u8,
    pub present: u8,
    pub dpl: u8,
    pub db: u8,
    pub s: u8,
    pub l: u8,
    pub g: u8,
    pub avl: u8,
    pub unusable: u8,
    #[allow(unused)]
    pub padding: u8,
}

/// Descriptor Table Register (for GDT, IDT)
#[derive(Clone, Debug, Default)]
pub struct Dtable {
    pub base: u64,
    pub limit: u16,
    #[allow(unused)]
    pub padding: [u16; 3],
}

#[derive(Debug, Default, Clone)]
pub struct VcpuSregs {
    pub cs: Segment,
    pub ds: Segment,
    pub es: Segment,
    pub fs: Segment,
    pub gs: Segment,
    pub ss: Segment,
    pub tr: Segment,
    pub ldt: Segment,
    pub gdt: Dtable,
    pub idt: Dtable,
    pub cr0: u64,
    pub cr2: u64,
    pub cr3: u64,
    pub cr4: u64,
    pub cr8: u64,
    pub efer: u64,
    pub apic_base: u64,
    #[allow(unused)]
    pub interrupt_bitmap: [u64; 4],
}
