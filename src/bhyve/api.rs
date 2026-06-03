#![allow(unused)]

use libc::c_char;

pub const VMM_CURRENT_INTERFACE_VERSION: i32 = 18;

pub const VMMCTL_IOC_BASE: u32 = (('V' as u32) << 16) | (('M' as u32) << 8);
pub const VMM_IOC_BASE: u32 = (('v' as u32) << 16) | (('m' as u32) << 8);
pub const VMM_LOCK_IOC_BASE: u32 = (('v' as u32) << 16) | (('l' as u32) << 8);
pub const VMM_CPU_IOC_BASE: u32 = (('v' as u32) << 16) | (('p' as u32) << 8);

/* Operations performed on the vmmctl device */
pub const VMM_CREATE_VM: u32 = VMMCTL_IOC_BASE | 0x01;
pub const VMM_DESTROY_VM: u32 = VMMCTL_IOC_BASE | 0x02;
pub const VMM_VM_SUPPORTED: u32 = VMMCTL_IOC_BASE | 0x03;
pub const VMM_INTERFACE_VERSION: u32 = VMMCTL_IOC_BASE | 0x04;
pub const VMM_CHECK_IOMMU: u32 = VMMCTL_IOC_BASE | 0x05;

pub const VMM_RESV_QUERY: u32 = VMMCTL_IOC_BASE | 0x10;
pub const VMM_RESV_SET_TARGET: u32 = VMMCTL_IOC_BASE | 0x11;

/* Operations performed in the context of a given vCPU */
pub const VM_RUN: u32 = VMM_CPU_IOC_BASE | 0x01;
pub const VM_SET_REGISTER: u32 = VMM_CPU_IOC_BASE | 0x02;
pub const VM_GET_REGISTER: u32 = VMM_CPU_IOC_BASE | 0x03;
pub const VM_SET_SEGMENT_DESCRIPTOR: u32 = VMM_CPU_IOC_BASE | 0x04;
pub const VM_GET_SEGMENT_DESCRIPTOR: u32 = VMM_CPU_IOC_BASE | 0x05;
pub const VM_SET_REGISTER_SET: u32 = VMM_CPU_IOC_BASE | 0x06;
pub const VM_GET_REGISTER_SET: u32 = VMM_CPU_IOC_BASE | 0x07;
pub const VM_INJECT_EXCEPTION: u32 = VMM_CPU_IOC_BASE | 0x08;
pub const VM_SET_CAPABILITY: u32 = VMM_CPU_IOC_BASE | 0x09;
pub const VM_GET_CAPABILITY: u32 = VMM_CPU_IOC_BASE | 0x0a;
pub const VM_PPTDEV_MSI: u32 = VMM_CPU_IOC_BASE | 0x0b;
pub const VM_PPTDEV_MSIX: u32 = VMM_CPU_IOC_BASE | 0x0c;
pub const VM_SET_X2APIC_STATE: u32 = VMM_CPU_IOC_BASE | 0x0d;
pub const VM_GLA2GPA: u32 = VMM_CPU_IOC_BASE | 0x0e;
pub const VM_GLA2GPA_NOFAULT: u32 = VMM_CPU_IOC_BASE | 0x0f;
pub const VM_ACTIVATE_CPU: u32 = VMM_CPU_IOC_BASE | 0x10;
pub const VM_SET_INTINFO: u32 = VMM_CPU_IOC_BASE | 0x11;
pub const VM_GET_INTINFO: u32 = VMM_CPU_IOC_BASE | 0x12;
pub const VM_RESTART_INSTRUCTION: u32 = VMM_CPU_IOC_BASE | 0x13;
pub const VM_SET_KERNEMU_DEV: u32 = VMM_CPU_IOC_BASE | 0x14;
pub const VM_GET_KERNEMU_DEV: u32 = VMM_CPU_IOC_BASE | 0x15;
pub const VM_RESET_CPU: u32 = VMM_CPU_IOC_BASE | 0x16;
pub const VM_GET_RUN_STATE: u32 = VMM_CPU_IOC_BASE | 0x17;
pub const VM_SET_RUN_STATE: u32 = VMM_CPU_IOC_BASE | 0x18;
pub const VM_GET_FPU: u32 = VMM_CPU_IOC_BASE | 0x19;
pub const VM_SET_FPU: u32 = VMM_CPU_IOC_BASE | 0x1a;
pub const VM_GET_CPUID: u32 = VMM_CPU_IOC_BASE | 0x1b;
pub const VM_SET_CPUID: u32 = VMM_CPU_IOC_BASE | 0x1c;
pub const VM_LEGACY_CPUID: u32 = VMM_CPU_IOC_BASE | 0x1d;

/* Operations requiring write-locking the VM */
pub const VM_REINIT: u32 = VMM_LOCK_IOC_BASE | 0x01;
pub const VM_BIND_PPTDEV: u32 = VMM_LOCK_IOC_BASE | 0x02;
pub const VM_UNBIND_PPTDEV: u32 = VMM_LOCK_IOC_BASE | 0x03;
pub const VM_MAP_PPTDEV_MMIO: u32 = VMM_LOCK_IOC_BASE | 0x04;
pub const VM_ALLOC_MEMSEG: u32 = VMM_LOCK_IOC_BASE | 0x05;
pub const VM_MMAP_MEMSEG: u32 = VMM_LOCK_IOC_BASE | 0x06;
pub const VM_PMTMR_LOCATE: u32 = VMM_LOCK_IOC_BASE | 0x07;
pub const VM_MUNMAP_MEMSEG: u32 = VMM_LOCK_IOC_BASE | 0x08;
pub const VM_UNMAP_PPTDEV_MMIO: u32 = VMM_LOCK_IOC_BASE | 0x09;
pub const VM_PAUSE: u32 = VMM_LOCK_IOC_BASE | 0x0a;
pub const VM_RESUME: u32 = VMM_LOCK_IOC_BASE | 0x0b;

pub const VM_WRLOCK_CYCLE: u32 = VMM_LOCK_IOC_BASE | 0xff;

/* All other ioctls */
pub const VM_GET_GPA_PMAP: u32 = VMM_IOC_BASE | 0x01;
pub const VM_GET_MEMSEG: u32 = VMM_IOC_BASE | 0x02;
pub const VM_MMAP_GETNEXT: u32 = VMM_IOC_BASE | 0x03;

pub const VM_LAPIC_IRQ: u32 = VMM_IOC_BASE | 0x04;
pub const VM_LAPIC_LOCAL_IRQ: u32 = VMM_IOC_BASE | 0x05;
pub const VM_LAPIC_MSI: u32 = VMM_IOC_BASE | 0x06;

pub const VM_IOAPIC_ASSERT_IRQ: u32 = VMM_IOC_BASE | 0x07;
pub const VM_IOAPIC_DEASSERT_IRQ: u32 = VMM_IOC_BASE | 0x08;
pub const VM_IOAPIC_PULSE_IRQ: u32 = VMM_IOC_BASE | 0x09;

pub const VM_ISA_ASSERT_IRQ: u32 = VMM_IOC_BASE | 0x0a;
pub const VM_ISA_DEASSERT_IRQ: u32 = VMM_IOC_BASE | 0x0b;
pub const VM_ISA_PULSE_IRQ: u32 = VMM_IOC_BASE | 0x0c;
pub const VM_ISA_SET_IRQ_TRIGGER: u32 = VMM_IOC_BASE | 0x0d;

pub const VM_RTC_WRITE: u32 = VMM_IOC_BASE | 0x0e;
pub const VM_RTC_READ: u32 = VMM_IOC_BASE | 0x0f;
pub const VM_RTC_SETTIME: u32 = VMM_IOC_BASE | 0x10;
pub const VM_RTC_GETTIME: u32 = VMM_IOC_BASE | 0x11;

pub const VM_SUSPEND: u32 = VMM_IOC_BASE | 0x12;

pub const VM_IOAPIC_PINCOUNT: u32 = VMM_IOC_BASE | 0x13;
pub const VM_GET_PPTDEV_LIMITS: u32 = VMM_IOC_BASE | 0x14;
pub const VM_GET_HPET_CAPABILITIES: u32 = VMM_IOC_BASE | 0x15;

pub const VM_STATS_IOC: u32 = VMM_IOC_BASE | 0x16;
pub const VM_STAT_DESC: u32 = VMM_IOC_BASE | 0x17;

pub const VM_INJECT_NMI: u32 = VMM_IOC_BASE | 0x18;
pub const VM_GET_X2APIC_STATE: u32 = VMM_IOC_BASE | 0x19;
pub const VM_SET_TOPOLOGY: u32 = VMM_IOC_BASE | 0x1a;
pub const VM_GET_TOPOLOGY: u32 = VMM_IOC_BASE | 0x1b;
pub const VM_GET_CPUS: u32 = VMM_IOC_BASE | 0x1c;
pub const VM_SUSPEND_CPU: u32 = VMM_IOC_BASE | 0x1d;
pub const VM_RESUME_CPU: u32 = VMM_IOC_BASE | 0x1e;

pub const VM_PPTDEV_DISABLE_MSIX: u32 = VMM_IOC_BASE | 0x1f;

/* Note: forces a barrier on a flush operation before returning. */
pub const VM_TRACK_DIRTY_PAGES: u32 = VMM_IOC_BASE | 0x20;
pub const VM_DESC_FPU_AREA: u32 = VMM_IOC_BASE | 0x21;

pub const VM_DATA_READ: u32 = VMM_IOC_BASE | 0x22;
pub const VM_DATA_WRITE: u32 = VMM_IOC_BASE | 0x23;

pub const VM_SET_AUTODESTRUCT: u32 = VMM_IOC_BASE | 0x24;
pub const VM_DESTROY_SELF: u32 = VMM_IOC_BASE | 0x25;
pub const VM_DESTROY_PENDING: u32 = VMM_IOC_BASE | 0x26;

pub const VM_VCPU_BARRIER: u32 = VMM_IOC_BASE | 0x27;
pub const VM_NPT_OPERATION: u32 = VMM_IOC_BASE | 0x28;

pub const VM_DEVMEM_GETOFFSET: u32 = VMM_IOC_BASE | 0xff;

pub const VMM_CTL_DEV: &str = "/dev/vmmctl";

// structs for ioctl calls

pub const VM_MAX_NAMELEN: usize = 128;
pub const VM_MAX_SEG_NAMELEN: usize = 128;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VmMemmap {
    pub gpa: u64,
    pub segid: libc::c_int,
    pub segoff: i64,
    pub len: libc::size_t,
    pub prot: libc::c_int,
    pub flags: libc::c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VmMemseg {
    pub segid: libc::c_int,
    pub len: libc::size_t,
    pub name: [libc::c_char; VM_MAX_SEG_NAMELEN],
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VmCreateReq {
    pub name: [c_char; VM_MAX_NAMELEN],
    pub flags: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VmDestroyReq {
    pub name: [c_char; VM_MAX_NAMELEN],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmRegName {
    Rax = 0,
    Rbx = 1,
    Rcx = 2,
    Rdx = 3,
    Rsi = 4,
    Rdi = 5,
    Rbp = 6,
    R8 = 7,
    R9 = 8,
    R10 = 9,
    R11 = 10,
    R12 = 11,
    R13 = 12,
    R14 = 13,
    R15 = 14,
    Cr0 = 15,
    Cr3 = 16,
    Cr4 = 17,
    Dr7 = 18,
    Rsp = 19,
    Rip = 20,
    Rflags = 21,
    Es = 22,
    Cs = 23,
    Ss = 24,
    Ds = 25,
    Fs = 26,
    Gs = 27,
    Ldtr = 28,
    Tr = 29,
    Idtr = 30,
    Gdtr = 31,
    Efer = 32,
    Cr2 = 33,
    Pdpte0 = 34,
    Pdpte1 = 35,
    Pdpte2 = 36,
    Pdpte3 = 37,
    IntrShadow = 38,
    Dr0 = 39,
    Dr1 = 40,
    Dr2 = 41,
    Dr3 = 42,
    Dr6 = 43,
    EntryInstLength = 44,
    Xcr0 = 45,
    Last = 46,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VmRegister {
    pub cpuid: libc::c_int,
    pub regnum: libc::c_int,
    pub regval: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SegDesc {
    pub base: u64,
    pub limit: u32,
    pub access: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VmSegDesc {
    pub cpuid: libc::c_int,
    pub regnum: libc::c_int,
    pub desc: SegDesc,
}

pub const VRS_HALT: u32 = 0;
pub const VRS_INIT: u32 = 1 << 0;
pub const VRS_RUN: u32 = 1 << 1;

pub const VRK_RESET: u32 = 0;
pub const VRK_INIT: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VmActivateCpu {
    pub vcpuid: libc::c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VmIsaIrq {
    pub atpic_irq: libc::c_int,
    pub ioapic_irq: libc::c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VmVcpuReset {
    pub vcpuid: libc::c_int,
    pub kind: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VmRunState {
    pub vcpuid: libc::c_int,
    pub state: u32,
    pub sipi_vector: u8,
    pub _pad: [u8; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VmGla2Gpa {
    pub vcpuid: libc::c_int,
    pub prot: libc::c_int,
    pub gla: u64,
    pub paging: VmGuestPaging,
    pub fault: libc::c_int,
    pub gpa: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmExitCode {
    Inout = 0,
    Vmx = 1,
    Bogus = 2,
    Rdmsr = 3,
    Wrmsr = 4,
    Hlt = 5,
    Mtrap = 6,
    Pause = 7,
    Paging = 8,
    InstEmul = 9,
    RunState = 10,
    MmioEmul = 11,
    Deprecated = 12,
    IoapicEoi = 13,
    Suspended = 14,
    Mmio = 15,
    TaskSwitch = 16,
    Monitor = 17,
    Mwait = 18,
    Svm = 19,
    Deprecated2 = 20,
    Debug = 21,
    Vminsn = 22,
    Bpt = 23,
    Ht = 24,
    Max = 25,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VmInout {
    pub eax: u32,
    pub port: u16,
    pub bytes: u8,
    pub flags: u8,
    pub addrsize: u8,
    pub segment: u8,
}

pub const INOUT_IN: u8 = 1 << 0;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VmMmio {
    pub bytes: u8,
    pub read: u8,
    pub _pad: [u16; 3],
    pub gpa: u64,
    pub data: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmCpuMode {
    Real = 0,
    Protected = 1,
    Compatibility = 2,
    SixtyFourBit = 3,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmPagingMode {
    Flat = 0,
    ThirtyTwo = 1,
    Pae = 2,
    SixtyFour = 3,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VmGuestPaging {
    pub cr3: u64,
    pub cpl: libc::c_int,
    pub cpu_mode: VmCpuMode,
    pub paging_mode: VmPagingMode,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VmTaskSwitch {
    pub tsssel: u16,
    pub ext: libc::c_int,
    pub errcode: u32,
    pub errcode_valid: libc::c_int,
    pub reason: libc::c_int,
    pub paging: VmGuestPaging,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union VmExitUnion {
    pub inout: VmInout,
    pub mmio: VmMmio,
    pub paging: VmExitPaging,
    pub mmio_emul: VmExitMmioEmul,
    pub inst_emul: VmExitInstEmul,
    pub vmx: VmExitVmx,
    pub svm: VmExitSvm,
    pub bpt: VmExitBpt,
    pub msr: VmExitMsr,
    pub hlt: VmExitHlt,
    pub ioapic_eoi: VmExitIoapicEoi,
    pub suspended: VmExitSuspended,
    pub task_switch: VmTaskSwitch,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VmExitPaging {
    pub gpa: u64,
    pub fault_type: libc::c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VmExitMmioEmul {
    pub gpa: u64,
    pub gla: u64,
    pub cs_base: u64,
    pub cs_d: libc::c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VmExitInstEmul {
    pub inst: [u8; 15],
    pub num_valid: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VmExitVmx {
    pub status: libc::c_int,
    pub exit_reason: u32,
    pub exit_qualification: u64,
    pub inst_type: libc::c_int,
    pub inst_error: libc::c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VmExitSvm {
    pub exitcode: u64,
    pub exitinfo1: u64,
    pub exitinfo2: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VmExitBpt {
    pub inst_length: libc::c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VmExitMsr {
    pub code: u32,
    pub wval: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VmExitHlt {
    pub rflags: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VmExitIoapicEoi {
    pub vector: libc::c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VmExitSuspended {
    pub how: libc::c_int,
    pub source: libc::c_int,
    pub when: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VmExit {
    pub exitcode: VmExitCode,
    pub inst_length: libc::c_int,
    pub rip: u64,
    pub u: VmExitUnion,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union VmEntryUnion {
    pub inout: VmInout,
    pub mmio: VmMmio,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VmEntry {
    pub cpuid: libc::c_int,
    pub cmd: u32,
    pub exit_data: *mut VmExit,
    pub u: VmEntryUnion,
}

pub const VEC_DEFAULT: u32 = 0;
pub const VEC_FULFILL_MMIO: u32 = 2;
pub const VEC_FULFILL_INOUT: u32 = 3;

#[repr(C)]
#[derive(Clone, Debug)]
pub struct VmRegisterSet {
    pub cpuid: libc::c_int,
    pub count: libc::c_uint,
    pub regnums: *const libc::c_int,
    pub regvals: *mut u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmCapType {
    HaltExit = 0,
    MtrapExit = 1,
    PauseExit = 2,
    EnableInvpcid = 3,
    BptExit = 4,
    Max = 5,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VmCapability {
    pub cpuid: libc::c_int,
    pub captype: VmCapType,
    pub capval: libc::c_int,
    pub allcpus: libc::c_int,
}
