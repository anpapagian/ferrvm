use super::ioctl::{io, ior, iow, iowr};

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct KvmIrqLevel {
    pub irq: u32,
    pub level: u32,
}

/// KVM Segment Register (for GDT, LDT, TR, etc.)
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KvmSegment {
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
    pub padding: u8,
}

/// Descriptor Table Register (for GDT, IDT)
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KvmDtable {
    pub base: u64,
    pub limit: u16,
    pub padding: [u16; 3],
}

/// KVM Special Registers (sregs)
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KvmSregs {
    pub cs: KvmSegment,
    pub ds: KvmSegment,
    pub es: KvmSegment,
    pub fs: KvmSegment,
    pub gs: KvmSegment,
    pub ss: KvmSegment,
    pub tr: KvmSegment,
    pub ldt: KvmSegment,
    pub gdt: KvmDtable,
    pub idt: KvmDtable,
    pub cr0: u64,
    pub cr2: u64,
    pub cr3: u64,
    pub cr4: u64,
    pub cr8: u64,
    pub efer: u64,
    pub apic_base: u64,
    pub interrupt_bitmap: [u64; 4], // (KVM_NR_INTERRUPTS + 63) / 64
}

/// KVM General-Purpose Registers (regs)
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KvmRegs {
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

/// KVM User Memory Region
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KvmUserspaceMemoryRegion {
    pub slot: u32,
    pub flags: u32,
    pub guest_phys_addr: u64,
    pub memory_size: u64,
    pub userspace_addr: u64,
}

pub const KVM_PIT_SPEAKER_DUMMY: u32 = 1;

/// PIT (Programmable Interval Timer) configuration
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KvmPitConfig {
    pub flags: u32,
    pub pad: [u32; 15],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KvmRun {
    /* in */
    pub request_interrupt_window: u8,
    pub immediate_exit: u8,
    pub padding1: [u8; 6],

    /* out */
    pub exit_reason: u32,
    pub ready_for_interrupt_injection: u8,
    pub if_flag: u8,
    pub flags: u16,

    /* in (pre_kvm_run), out (post_kvm_run) */
    pub cr8: u64,
    pub apic_base: u64,

    pub exit: KvmRunExit,
    // some more things here (i.e. kvm_valid_regs/kvm_dirty_regs etc.)
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union KvmRunExit {
    pub hw: KvmExitUnknown,           // KVM_EXIT_UNKNOWN
    pub fail_entry: KvmExitFailEntry, // KVM_EXIT_FAIL_ENTRY
    pub ex: KvmExitException,         // KVM_EXIT_EXCEPTION
    pub io: KvmExitIo,                // KVM_EXIT_IO
    pub debug: KvmExitDebug,          // KVM_EXIT_DEBUG
    pub mmio: KvmExitMmio,            // KVM_EXIT_MMIO
    // KVM_EXIT_LOONGARCH_IOCSR
    // KVM_EXIT_HYPERCALL
    // KVM_EXIT_TPR_ACCESS
    // KVM_EXIT_S390_SIEIC
    // KVM_EXIT_S390_RESET
    // KVM_EXIT_S390_UCONTROL
    // KVM_EXIT_DCR (deprecated)
    // KVM_EXIT_INTERNAL_ERROR
    // KVM_INTERNAL_ERROR_EMULATION
    // KVM_EXIT_OSI
    // KVM_EXIT_PAPR_HCALL
    // KVM_EXIT_S390_TSCH
    // KVM_EXIT_EPR
    // KVM_EXIT_SYSTEM_EVENT
    // KVM_EXIT_S390_STSI
    // KVM_EXIT_IOAPIC_EOI
    // KVM_EXIT_HYPERV
    // KVM_EXIT_X86_RDMSR / KVM_EXIT_X86_WRMSR
    // KVM_EXIT_XEN
    // KVM_EXIT_RISCV_SBI
    // KVM_EXIT_RISCV_CSR
    // KVM_EXIT_NOTIFY
    // KVM_EXIT_MEMORY_FAULT
    // KVM_EXIT_TDX
    // KVM_EXIT_ARM_SEA
    pub padding: [u8; 256], // padding to make the struct large enough for all exit types
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KvmExitUnknown {
    pub hardware_exit_reason: u64,
}

/// Contains hardware error code when vCPU fails to enter guest mode
#[repr(C)]
#[derive(Clone, Copy)]
pub struct KvmExitFailEntry {
    /// VMX/SVM hardware error code
    /// For VMX: this is the VM-instruction error field from the VMCS
    /// For SVM: this is the error code from the VMCB
    pub hardware_entry_failure_reason: u64,
    /// CPU index that failed
    pub cpu: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KvmExitException {
    pub exception: u32,
    pub error_code: u32,
}

pub const KVM_EXIT_IO_IN: u8 = 0;
pub const KVM_EXIT_IO_OUT: u8 = 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KvmExitIo {
    pub direction: u8, /* KVM_EXIT_IO_IN or KVM_EXIT_IO_OUT */
    pub size: u8,
    pub port: u16,
    pub count: u32,
    pub data_offset: u64, /* relative to kvm_run start */
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KvmDebugExitArch {
    pub exception: u32,
    pub pad: u32,
    pub pc: u64,
    pub dr6: u64,
    pub dr7: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KvmExitDebug {
    pub arch: KvmDebugExitArch,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KvmExitMmio {
    pub phys_addr: u64,
    pub data: [u8; 8],
    pub len: u32,
    pub is_write: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KvmCpuidEntry2 {
    pub function: u32,
    pub index: u32,
    pub flags: u32,
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
    pub padding: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KvmCpuid2 {
    pub nent: u32,
    pub padding: u32,
    pub entries: [KvmCpuidEntry2; 256],
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KvmCpuid2Empty {
    pub nent: u32,
    pub padding: u32,
}

/// VM exit reason enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmExitReason {
    Unknown = 0,
    Exception = 1,
    Io = 2,
    Hypercall = 3,
    Debug = 4,
    Hlt = 5,
    MmIo = 6,
    IrqWindowOpen = 7,
    Shutdown = 8,
    FailEntry = 9,
    Intr = 10,
    SetTpr = 11,
    TprAccess = 12,
    S390Sieic = 13,
    S390Reset = 14,
    Dcr = 15, // deprecated, but we include it for completeness
    Nmi = 16,
    InternalError = 17,
    Osi = 18,
    PaprHcall = 19,
    S390Ucontrol = 20,
    Watchdog = 21,
    S390Tsch = 22,
    Epr = 23,
    SystemEvent = 24,
    S390Stsi = 25,
    IoapicEoi = 26,
    Hyperv = 27,
    ArmNisv = 28,
    X86Rdmsr = 29,
    X86Wrmsr = 30,
    DirtyRingFull = 31,
    ApResetHold = 32,
    X86BusLock = 33,
    Xen = 34,
    RiscvSbi = 35,
    RiscvCsr = 36,
    Notify = 37,
    LoongarchIocsr = 38,
    MemoryFault = 39,
    Tdx = 40,
    ArmSea = 41,
}

impl TryFrom<u32> for VmExitReason {
    type Error = u32;

    fn try_from(val: u32) -> Result<Self, Self::Error> {
        match val {
            0 => Ok(Self::Unknown),
            1 => Ok(Self::Exception),
            2 => Ok(Self::Io),
            3 => Ok(Self::Hypercall),
            4 => Ok(Self::Debug),
            5 => Ok(Self::Hlt),
            6 => Ok(Self::MmIo),
            7 => Ok(Self::IrqWindowOpen),
            8 => Ok(Self::Shutdown),
            9 => Ok(Self::FailEntry),
            10 => Ok(Self::Intr),
            11 => Ok(Self::SetTpr),
            12 => Ok(Self::TprAccess),
            13 => Ok(Self::S390Sieic),
            14 => Ok(Self::S390Reset),
            15 => Ok(Self::Dcr),
            16 => Ok(Self::Nmi),
            17 => Ok(Self::InternalError),
            18 => Ok(Self::Osi),
            19 => Ok(Self::PaprHcall),
            20 => Ok(Self::S390Ucontrol),
            21 => Ok(Self::Watchdog),
            22 => Ok(Self::S390Tsch),
            23 => Ok(Self::Epr),
            24 => Ok(Self::SystemEvent),
            25 => Ok(Self::S390Stsi),
            26 => Ok(Self::IoapicEoi),
            27 => Ok(Self::Hyperv),
            28 => Ok(Self::ArmNisv),
            29 => Ok(Self::X86Rdmsr),
            30 => Ok(Self::X86Wrmsr),
            31 => Ok(Self::DirtyRingFull),
            32 => Ok(Self::ApResetHold),
            33 => Ok(Self::X86BusLock),
            34 => Ok(Self::Xen),
            35 => Ok(Self::RiscvSbi),
            36 => Ok(Self::RiscvCsr),
            37 => Ok(Self::Notify),
            38 => Ok(Self::LoongarchIocsr),
            39 => Ok(Self::MemoryFault),
            40 => Ok(Self::Tdx),
            41 => Ok(Self::ArmSea),
            _ => Err(val),
        }
    }
}

impl std::fmt::Display for VmExitReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown => write!(f, "Unknown"),
            Self::Exception => write!(f, "Exception"),
            Self::Io => write!(f, "IO"),
            Self::Hypercall => write!(f, "Hypercall"),
            Self::Debug => write!(f, "Debug"),
            Self::Hlt => write!(f, "Hlt"),
            Self::MmIo => write!(f, "MMIO"),
            Self::IrqWindowOpen => write!(f, "IRQ Window Open"),
            Self::Shutdown => write!(f, "Shutdown"),
            Self::FailEntry => write!(f, "Fail Entry"),
            Self::Intr => write!(f, "Interrupt"),
            Self::SetTpr => write!(f, "Set TPR"),
            Self::TprAccess => write!(f, "TPR Access"),
            Self::S390Sieic => write!(f, "S390 Sieic"),
            Self::S390Reset => write!(f, "S390 Reset"),
            Self::Dcr => write!(f, "DCR"),
            Self::Nmi => write!(f, "NMI"),
            Self::InternalError => write!(f, "Internal Error"),
            Self::Osi => write!(f, "OSI"),
            Self::PaprHcall => write!(f, "PAPR Hypercall"),
            Self::S390Ucontrol => write!(f, "S390 Ucontrol"),
            Self::Watchdog => write!(f, "Watchdog"),
            Self::S390Tsch => write!(f, "S390 Tsch"),
            Self::Epr => write!(f, "EPR"),
            Self::SystemEvent => write!(f, "System Event"),
            Self::S390Stsi => write!(f, "S390 Stsi"),
            Self::IoapicEoi => write!(f, "IOAPIC EOI"),
            Self::Hyperv => write!(f, "Hyper-V"),
            Self::ArmNisv => write!(f, "ARM NISV"),
            Self::X86Rdmsr => write!(f, "x86 RDMSR"),
            Self::X86Wrmsr => write!(f, "x86 WRMSR"),
            Self::DirtyRingFull => write!(f, "Dirty Ring Full"),
            Self::ApResetHold => write!(f, "AP Reset Hold"),
            Self::X86BusLock => write!(f, "x86 Bus Lock"),
            Self::Xen => write!(f, "Xen"),
            Self::RiscvSbi => write!(f, "RISC-V SBI"),
            Self::RiscvCsr => write!(f, "RISC-V CSR"),
            Self::Notify => write!(f, "Notify"),
            Self::LoongarchIocsr => write!(f, "LoongArch IOCSR"),
            Self::MemoryFault => write!(f, "Memory Fault"),
            Self::Tdx => write!(f, "TDX"),
            Self::ArmSea => write!(f, "ARM SEA"),
        }
    }
}

pub const KVM_API_VERSION: i32 = 12;
pub const KVM_TSS_ADDRESS: usize = 0xfffb_d000;

const KVMIO: u32 = 0xAE;

// System ioctls (on /dev/kvm fd)
pub const KVM_GET_API_VERSION: u64 = io(KVMIO, 0x00);
pub const KVM_CREATE_VM: u64 = io(KVMIO, 0x01);
pub const KVM_GET_VCPU_MMAP_SIZE: u64 = io(KVMIO, 0x04);
pub const KVM_GET_SUPPORTED_CPUID: u64 = iowr::<KvmCpuid2Empty>(KVMIO, 0x05);

// VM ioctls (on VM fd)
pub const KVM_CREATE_VCPU: u64 = io(KVMIO, 0x41);
pub const KVM_SET_USER_MEMORY_REGION: u64 = iow::<KvmUserspaceMemoryRegion>(KVMIO, 0x46);
pub const KVM_SET_TSS_ADDR: u64 = io(KVMIO, 0x47);
pub const KVM_CREATE_IRQCHIP: u64 = io(KVMIO, 0x60);
pub const KVM_CREATE_PIT2: u64 = iow::<KvmPitConfig>(KVMIO, 0x77);
pub const KVM_IRQ_LINE: u64 = iow::<KvmIrqLevel>(KVMIO, 0x61);

// vCPU ioctls (on vCPU fd)
pub const KVM_RUN: u64 = io(KVMIO, 0x80);
pub const KVM_GET_REGS: u64 = ior::<KvmRegs>(KVMIO, 0x81);
pub const KVM_SET_REGS: u64 = iow::<KvmRegs>(KVMIO, 0x82);
pub const KVM_GET_SREGS: u64 = ior::<KvmSregs>(KVMIO, 0x83);
pub const KVM_SET_SREGS: u64 = iow::<KvmSregs>(KVMIO, 0x84);
pub const KVM_SET_CPUID2: u64 = iow::<KvmCpuid2Empty>(KVMIO, 0x90);
