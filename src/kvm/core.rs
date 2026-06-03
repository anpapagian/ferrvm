use std::fs::File;
use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::ptr::NonNull;
use std::sync::Arc;

use super::api;
use super::api::{
    KVM_API_VERSION, KVM_CREATE_IRQCHIP, KVM_CREATE_PIT2, KVM_CREATE_VCPU, KVM_CREATE_VM,
    KVM_GET_API_VERSION, KVM_GET_REGS, KVM_GET_SREGS, KVM_GET_SUPPORTED_CPUID,
    KVM_GET_VCPU_MMAP_SIZE, KVM_PIT_SPEAKER_DUMMY, KVM_RUN, KVM_SET_CPUID2, KVM_SET_REGS,
    KVM_SET_SREGS, KVM_SET_TSS_ADDR, KVM_SET_USER_MEMORY_REGION, KVM_TSS_ADDRESS, KvmCpuid2,
    KvmDtable, KvmExitIo, KvmPitConfig, KvmRegs, KvmRun, KvmSregs, KvmUserspaceMemoryRegion,
    VmExitReason,
};
use super::exithandler::handle_exit;
use super::ioctl::{ioctl_with_mut_ref, ioctl_with_ref, ioctl_with_val};
use super::irq;
use crate::traits::{self, Hypervisor, Vcpu, Vm};
use ferrvm::printcrln;

pub struct Kvm {
    fd: File,
}

impl Hypervisor for Kvm {
    fn create_vm<'a>(&'a self, _name: &str) -> traits::Result<Box<dyn Vm + 'a>> {
        let vm = self
            .create_vm()
            .map_err(|e| format!("Failed to create VM: {e}"))?;
        Ok(Box::new(vm))
    }
}

impl Vm for KvmVm<'_> {
    fn setup(&self) -> traits::Result<()> {
        printcrln!("[vm] Creating in-kernel IRQ chip (PIC/IOAPIC)...");
        self.create_irqchip()
            .map_err(|e| format!("Failed to create in-kernel IRQ chip (PIC/IOAPIC): {e}"))?;

        printcrln!("[vm] Creating in-kernel PIT (Programmable Interval Timer)...");
        self.create_pit2()
            .map_err(|e| format!("Failed to create in-kernel PIT timer: {e}"))?;

        self.set_tss_address(KVM_TSS_ADDRESS as u64)
            .map_err(|e| format!("Failed to set TSS address for vCPU: {e}"))?;

        Ok(())
    }

    fn create_vcpu<'a>(&'a self, id: u32) -> traits::Result<Box<dyn Vcpu + Send + 'a>> {
        let vcpu = self
            .create_vcpu(id)
            .map_err(|e| format!("Failed to create vCPU: {e}"))?;
        Ok(Box::new(vcpu))
    }

    fn register_memory_region(&self, gpa: u64, size: u64, hpa: u64) -> traits::Result<()> {
        self.set_user_memory_region(0, gpa, size, hpa)
            .map_err(|e| format!("Failed to register memory region with KVM: {e}").into())
    }

    fn register_irq(&self, irq: u32) -> traits::Result<Arc<dyn traits::IrqSink>> {
        Ok(Arc::new(irq::KvmIrqSink::new(self.fd.as_raw_fd(), irq)))
    }
}

impl From<&traits::Segment> for api::KvmSegment {
    fn from(seg: &traits::Segment) -> Self {
        api::KvmSegment {
            base: seg.base,
            limit: seg.limit,
            selector: seg.selector,
            type_: seg.type_,
            present: seg.present,
            dpl: seg.dpl,
            db: seg.db,
            s: seg.s,
            l: seg.l,
            g: seg.g,
            avl: seg.avl,
            unusable: seg.unusable,
            padding: seg.padding,
        }
    }
}

impl From<&api::KvmSegment> for traits::Segment {
    fn from(seg: &api::KvmSegment) -> Self {
        traits::Segment {
            base: seg.base,
            limit: seg.limit,
            selector: seg.selector,
            type_: seg.type_,
            present: seg.present,
            dpl: seg.dpl,
            db: seg.db,
            s: seg.s,
            l: seg.l,
            g: seg.g,
            avl: seg.avl,
            unusable: seg.unusable,
            padding: seg.padding,
        }
    }
}

impl From<&KvmDtable> for traits::Dtable {
    fn from(dtable: &KvmDtable) -> Self {
        traits::Dtable {
            base: dtable.base,
            limit: dtable.limit,
            padding: dtable.padding,
        }
    }
}

impl From<&traits::Dtable> for KvmDtable {
    fn from(dtable: &traits::Dtable) -> Self {
        KvmDtable {
            base: dtable.base,
            limit: dtable.limit,
            padding: dtable.padding,
        }
    }
}

impl Vcpu for KvmVcpu<'_> {
    fn setup(&self) -> traits::Result<()> {
        // Get supported CPUID entries from the host
        printcrln!("[boot] Getting supported CPUID entries from host");
        let cpuid_entries = self
            .kvm
            .get_supported_cpuid()
            .map_err(|e| format!("Failed to get supported CPUID: {e}"))?;

        printcrln!("[boot] Setting CPUID entries on vCPU");
        self.set_cpuid2(&cpuid_entries)
            .map_err(|e| format!("Failed to set CPUID entries on vCPU: {e}"))?;

        printcrln!("[boot] CPUID entries set successfully");

        Ok(())
    }

    fn step_run(
        &mut self,
        stats: &mut crate::stats::ExitStats,
        _memory: &crate::memory::GuestMemory,
        pio_bus: &crate::bus::Bus,
        mmio_bus: &crate::bus::Bus,
        config: &crate::config::Config,
    ) -> core::result::Result<bool, std::io::Error> {
        // Run the vCPU until the next vm exit. EINTR means a SIGUSR1 kick
        // from `Shutdown::request()` popped us out of KVM_RUN — loop back
        // so the flag check above can observe it.
        match self.run() {
            Ok(r) => {
                Ok(handle_exit(r, stats, self, pio_bus, mmio_bus, config)
                    .map_err(io::Error::other)?)
            }
            Err(e) => Err(e),
        }
    }

    fn set_regs(&self, regs: &traits::VcpuRegs) -> traits::Result<()> {
        match self.set_regs(&api::KvmRegs {
            rax: regs.rax,
            rbx: regs.rbx,
            rcx: regs.rcx,
            rdx: regs.rdx,
            rsi: regs.rsi,
            rdi: regs.rdi,
            rsp: regs.rsp,
            rbp: regs.rbp,
            r8: regs.r8,
            r9: regs.r9,
            r10: regs.r10,
            r11: regs.r11,
            r12: regs.r12,
            r13: regs.r13,
            r14: regs.r14,
            r15: regs.r15,
            rip: regs.rip,
            rflags: regs.rflags,
        }) {
            Ok(()) => Ok(()),
            Err(e) => Err(format!("Failed to set vCPU registers: {e}").into()),
        }
    }

    fn get_regs(&self) -> traits::Result<traits::VcpuRegs> {
        match self.get_regs() {
            Ok(regs) => Ok(traits::VcpuRegs {
                rax: regs.rax,
                rbx: regs.rbx,
                rcx: regs.rcx,
                rdx: regs.rdx,
                rsi: regs.rsi,
                rdi: regs.rdi,
                rsp: regs.rsp,
                rbp: regs.rbp,
                r8: regs.r8,
                r9: regs.r9,
                r10: regs.r10,
                r11: regs.r11,
                r12: regs.r12,
                r13: regs.r13,
                r14: regs.r14,
                r15: regs.r15,
                rip: regs.rip,
                rflags: regs.rflags,
            }),
            Err(e) => Err(e.into()),
        }
    }

    fn set_sregs(&self, sregs: &traits::VcpuSregs) -> traits::Result<()> {
        match self.set_sregs(&KvmSregs {
            cs: (&sregs.cs).into(),
            ds: (&sregs.ds).into(),
            es: (&sregs.es).into(),
            fs: (&sregs.fs).into(),
            gs: (&sregs.gs).into(),
            ss: (&sregs.ss).into(),
            tr: (&sregs.tr).into(),
            ldt: (&sregs.ldt).into(),
            gdt: (&sregs.gdt).into(),
            idt: (&sregs.idt).into(),
            cr0: sregs.cr0,
            cr2: sregs.cr2,
            cr3: sregs.cr3,
            cr4: sregs.cr4,
            cr8: sregs.cr8,
            efer: sregs.efer,
            apic_base: sregs.apic_base,
            interrupt_bitmap: sregs.interrupt_bitmap,
        }) {
            Ok(()) => Ok(()),
            Err(e) => Err(format!("Failed to set vCPU special registers: {e}").into()),
        }
    }

    fn get_sregs(&self) -> traits::Result<traits::VcpuSregs> {
        match self.get_sregs() {
            Ok(sregs) => Ok(traits::VcpuSregs {
                cs: (&sregs.cs).into(),
                ds: (&sregs.ds).into(),
                es: (&sregs.es).into(),
                fs: (&sregs.fs).into(),
                gs: (&sregs.gs).into(),
                ss: (&sregs.ss).into(),
                tr: (&sregs.tr).into(),
                ldt: (&sregs.ldt).into(),
                gdt: (&sregs.gdt).into(),
                idt: (&sregs.idt).into(),
                cr0: sregs.cr0,
                cr2: sregs.cr2,
                cr3: sregs.cr3,
                cr4: sregs.cr4,
                cr8: sregs.cr8,
                efer: sregs.efer,
                apic_base: sregs.apic_base,
                interrupt_bitmap: sregs.interrupt_bitmap,
            }),
            Err(e) => Err(e.into()),
        }
    }
}

impl Kvm {
    pub fn new() -> Result<Self, String> {
        let kvm = Self::open()?;

        let api_version = kvm.get_api_version()?;
        printcrln!("[kvm] KVM API version: {api_version}");
        if api_version != KVM_API_VERSION {
            return Err(format!(
                "Unsupported KVM API version: got {api_version}, expected {KVM_API_VERSION}"
            ));
        }
        printcrln!("[kvm] API version is supported");
        Ok(kvm)
    }

    pub fn open() -> Result<Self, String> {
        let fd = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/kvm")
            .map_err(|e| {
                format!(
                    "Unable to open /dev/kvm (check permissions and that KVM is available): {e}"
                )
            })?;

        Ok(Self { fd })
    }

    pub fn get_api_version(&self) -> Result<i32, String> {
        let fd = self.fd.as_raw_fd();

        // SAFETY: fd is a valid /dev/kvm handle, KVM_GET_API_VERSION takes no argument.
        let ret = unsafe { ioctl_with_val(fd, KVM_GET_API_VERSION, 0) };
        match ret {
            Ok(val) => Ok(val),
            Err(e) => Err(format!("KVM_GET_API_VERSION ioctl failed: {e}")),
        }
    }

    pub fn create_vm(&self) -> Result<KvmVm<'_>, String> {
        let fd = self.fd.as_raw_fd();

        // SAFETY: fd is a valid /dev/kvm handle, KVM_CREATE_VM takes no argument.
        let vm_fd = match unsafe { ioctl_with_val(fd, KVM_CREATE_VM, 0) } {
            Ok(val) => val as RawFd,
            Err(e) => return Err(format!("KVM_CREATE_VM ioctl failed: {e}")),
        };

        // SAFETY: fd is a valid /dev/kvm handle, KVM_GET_VCPU_MMAP_SIZE takes no argument.
        let mmap_size = match unsafe { ioctl_with_val(fd, KVM_GET_VCPU_MMAP_SIZE, 0) } {
            Ok(size) => {
                printcrln!("[vm] kvm_run mmap size: {size} bytes");
                size as usize
            }
            Err(e) => return Err(format!("KVM_GET_VCPU_MMAP_SIZE ioctl failed: {e}")),
        };

        Ok(KvmVm {
            // SAFETY: vm_fd is a fresh VM fd returned by KVM_CREATE_VM, owned exclusively from here.
            fd: unsafe { std::os::unix::io::FromRawFd::from_raw_fd(vm_fd) },
            kvm: self,
            mmap_size,
        })
    }

    pub fn get_supported_cpuid(&self) -> Result<KvmCpuid2, String> {
        let fd = self.fd.as_raw_fd();

        let initial_nent = 256_u32;

        // Create the CPUID structure
        // SAFETY: KvmCpuid2 is plain old data, an all-zero bit pattern is a valid value.
        let mut cpuid: KvmCpuid2 = unsafe { std::mem::zeroed() };
        cpuid.nent = initial_nent;

        // Call the ioctl
        // SAFETY: fd is a valid /dev/kvm handle and cpuid is a correctly sized KvmCpuid2 with nent set.
        let ret = unsafe { ioctl_with_mut_ref(fd, KVM_GET_SUPPORTED_CPUID, &mut cpuid) };
        match ret {
            Ok(_) => (),
            Err(e) => return Err(format!("KVM_GET_SUPPORTED_CPUID ioctl failed: {e}")),
        }

        let returned_entries = usize::try_from(cpuid.nent)
            .map_err(|_| format!("KVM returned an invalid CPUID entry count: {}", cpuid.nent))?;

        if returned_entries > cpuid.entries.len() {
            return Err(format!(
                "KVM returned {} CPUID entries, but the buffer only holds {}",
                returned_entries,
                cpuid.entries.len()
            ));
        }

        printcrln!("[kvm] Got {} supported CPUID entries", cpuid.nent);

        Ok(cpuid)
    }
}

pub struct KvmVm<'a> {
    fd: File,
    kvm: &'a Kvm,
    mmap_size: usize,
}

impl AsRawFd for KvmVm<'_> {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl std::fmt::Debug for KvmVm<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "KvmVm {{ fd: {} }}", self.as_raw_fd())
    }
}

impl KvmRun {
    /// Get the exit reason as an enum. Returns `Err(code)` for unrecognised exit codes.
    pub fn exit_reason_enum(&self) -> Result<VmExitReason, u32> {
        VmExitReason::try_from(self.exit_reason)
    }

    /// Parse IO exit data
    pub const fn io_exit(&self) -> Option<KvmExitIo> {
        if self.exit_reason != VmExitReason::Io as u32 {
            return None;
        }

        // SAFETY: exit_reason was checked to be Io above, so the io union variant is active.
        let io = unsafe { self.exit.io };

        Some(io)
    }
}

impl KvmVm<'_> {
    pub fn set_user_memory_region(
        &self,
        slot: u32,
        guest_phys_addr: u64,
        size: u64,
        userspace_addr: u64,
    ) -> Result<(), String> {
        let region = KvmUserspaceMemoryRegion {
            slot,
            flags: 0,
            guest_phys_addr,
            memory_size: size,
            userspace_addr,
        };

        let fd = self.as_raw_fd();

        // SAFETY: fd is a valid VM fd and region is a correctly sized KvmUserspaceMemoryRegion.
        let ret = unsafe { ioctl_with_ref(fd, KVM_SET_USER_MEMORY_REGION, &region) };
        match ret {
            Ok(_) => Ok(()),
            Err(e) => Err(format!("KVM_SET_USER_MEMORY_REGION ioctl failed: {e}")),
        }
    }

    /// Create an in-kernel interrupt controller (PIC, IOAPIC, LAPIC)
    pub fn create_irqchip(&self) -> Result<(), String> {
        let fd = self.as_raw_fd();

        // SAFETY: fd is a valid VM fd, KVM_CREATE_IRQCHIP takes no argument.
        let ret = unsafe { ioctl_with_val(fd, KVM_CREATE_IRQCHIP, 0) };
        match ret {
            Ok(_) => Ok(()),
            Err(e) => Err(format!("KVM_CREATE_IRQCHIP ioctl failed: {e}")),
        }
    }

    pub fn set_tss_address(&self, addr: u64) -> Result<(), String> {
        let fd = self.as_raw_fd();

        // SAFETY: fd is a valid VM fd, KVM_SET_TSS_ADDR takes the address as its argument.
        let ret = unsafe { ioctl_with_val(fd, KVM_SET_TSS_ADDR, addr) };
        match ret {
            Ok(_) => Ok(()),
            Err(e) => Err(format!("KVM_SET_TSS_ADDR ioctl failed: {e}")),
        }
    }

    /// Create an in-kernel Programmable Interval Timer
    pub fn create_pit2(&self) -> Result<(), String> {
        let pit_config = KvmPitConfig {
            flags: KVM_PIT_SPEAKER_DUMMY,
            pad: [0; 15],
        };

        let fd = self.as_raw_fd();

        // SAFETY: fd is a valid VM fd and pit_config is a correctly sized KvmPitConfig for KVM_CREATE_PIT2.
        let ret = unsafe { ioctl_with_ref(fd, KVM_CREATE_PIT2, &pit_config) };
        match ret {
            Ok(_) => Ok(()),
            Err(e) => Err(format!("KVM_CREATE_PIT2 ioctl failed: {e}")),
        }
    }

    /// Create a vCPU
    pub fn create_vcpu(&self, vcpu_id: u32) -> Result<KvmVcpu<'_>, String> {
        let fd = self.as_raw_fd();

        // SAFETY: fd is a valid VM fd, KVM_CREATE_VCPU takes the vcpu id as its argument.
        let ret = unsafe { ioctl_with_val(fd, KVM_CREATE_VCPU, libc::c_ulong::from(vcpu_id)) };
        let fd = match ret {
            Ok(val) => val as RawFd, // ret is the new vCPU file descriptor
            Err(e) => return Err(format!("KVM_CREATE_VCPU ioctl failed: {e}")),
        };

        // SAFETY: fd is a fresh vCPU fd returned by KVM_CREATE_VCPU, owned exclusively from here.
        let file = unsafe { std::os::unix::io::FromRawFd::from_raw_fd(fd) };

        // Map the kvm_run structure
        // SAFETY: null hint, fd is the vCPU fd, mmap_size from KVM_GET_VCPU_MMAP_SIZE; result checked below.
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                self.mmap_size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };

        if ptr == libc::MAP_FAILED {
            let err = io::Error::last_os_error();
            return Err(format!("Failed to mmap kvm_run structure: {err}"));
        }

        printcrln!("[vcpu] Mapped kvm_run structure at {ptr:p}");

        let kvm_run = NonNull::new(ptr.cast::<KvmRun>())
            .ok_or_else(|| "mmap returned a null pointer for kvm_run".to_string())?;

        Ok(KvmVcpu {
            fd: file,
            kvm_run,
            mmap_size: self.mmap_size,
            kvm: self.kvm,
        })
    }
}

// SAFETY: a `KvmVcpu` exclusively owns its vCPU fd and `kvm_run` mmap, so moving
// it across threads can't alias the `kvm_run` pointer. KVM only requires a vCPU
// be driven from one thread at a time, which `Box<dyn Vcpu + Send>` upholds.
unsafe impl Send for KvmVcpu<'_> {}

/// `Send` but intentionally not `Sync`: the vCPU fd and `kvm_run` mapping are
/// owned by one thread at a time. It may be *moved* to its driver thread but not
/// *shared* — KVM forbids concurrent `KVM_RUN` on one fd, and `run(&self)` makes
/// the kernel write the shared `kvm_run` page with no userspace locking.
pub struct KvmVcpu<'a> {
    pub fd: File,
    pub kvm_run: NonNull<KvmRun>,
    pub mmap_size: usize,
    pub kvm: &'a Kvm,
}

impl AsRawFd for KvmVcpu<'_> {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl std::fmt::Debug for KvmVcpu<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Vcpu {{ fd: {} }}", self.as_raw_fd())
    }
}

impl KvmVcpu<'_> {
    /// Run the vCPU until the next VM exit.
    ///
    /// Returns `io::Result` (not `Result<_, String>`) specifically so the
    /// caller can distinguish `ErrorKind::Interrupted` — used by the vCPU
    /// loop to notice SIGUSR1 kicks from shutdown without treating them as
    /// fatal.
    pub fn run(&self) -> io::Result<&KvmRun> {
        let fd = self.as_raw_fd();
        // SAFETY: fd is a valid vCPU fd, KVM_RUN takes no argument.
        unsafe { ioctl_with_val(fd, KVM_RUN, 0) }?;
        // SAFETY: kvm_run points to the live mmap'd page, exclusively borrowed via &self.
        Ok(unsafe { self.kvm_run.as_ref() })
    }

    pub fn get_regs(&self) -> Result<KvmRegs, String> {
        let fd = self.as_raw_fd();
        // SAFETY: KvmRegs is plain old data, an all-zero bit pattern is a valid value.
        let mut regs: KvmRegs = unsafe { std::mem::zeroed() };

        // SAFETY: fd is a valid vCPU fd and regs is a correctly sized KvmRegs for KVM_GET_REGS.
        let ret = unsafe { ioctl_with_mut_ref(fd, KVM_GET_REGS, &mut regs) };
        match ret {
            Ok(_) => Ok(regs),
            Err(e) => Err(format!("KVM_GET_REGS ioctl failed: {e}")),
        }
    }

    pub fn set_regs(&self, regs: &KvmRegs) -> Result<(), String> {
        let fd = self.as_raw_fd();

        // SAFETY: fd is a valid vCPU fd and regs is a correctly sized KvmRegs for KVM_SET_REGS.
        let ret = unsafe { ioctl_with_ref(fd, KVM_SET_REGS, regs) };
        match ret {
            Ok(_) => Ok(()),
            Err(e) => Err(format!("KVM_SET_REGS ioctl failed: {e}")),
        }
    }

    pub fn get_sregs(&self) -> Result<KvmSregs, String> {
        let fd = self.as_raw_fd();
        // SAFETY: KvmSregs is plain old data, an all-zero bit pattern is a valid value.
        let mut sregs: KvmSregs = unsafe { std::mem::zeroed() };

        // SAFETY: fd is a valid vCPU fd and sregs is a correctly sized KvmSregs for KVM_GET_SREGS.
        let ret = unsafe { ioctl_with_mut_ref(fd, KVM_GET_SREGS, &mut sregs) };
        match ret {
            Ok(_) => Ok(sregs),
            Err(e) => Err(format!("KVM_GET_SREGS ioctl failed: {e}")),
        }
    }

    pub fn set_sregs(&self, sregs: &KvmSregs) -> Result<(), String> {
        let fd = self.as_raw_fd();

        // SAFETY: fd is a valid vCPU fd and sregs is a correctly sized KvmSregs for KVM_SET_SREGS.
        let ret = unsafe { ioctl_with_ref(fd, KVM_SET_SREGS, sregs) };
        match ret {
            Ok(_) => Ok(()),
            Err(e) => Err(format!("KVM_SET_SREGS ioctl failed: {e}")),
        }
    }

    pub fn set_cpuid2(&self, cpuid: &KvmCpuid2) -> Result<(), String> {
        let fd = self.as_raw_fd();

        // SAFETY: fd is a valid vCPU fd and cpuid is a correctly sized KvmCpuid2 for KVM_SET_CPUID2.
        let ret = unsafe { ioctl_with_ref(fd, KVM_SET_CPUID2, cpuid) };
        match ret {
            Ok(_) => Ok(()),
            Err(e) => Err(format!("KVM_SET_CPUID2 ioctl failed: {e}")),
        }
    }
}

impl Drop for KvmVcpu<'_> {
    fn drop(&mut self) {
        if self.mmap_size > 0 {
            // SAFETY: ptr and size come from the kvm_run mmap in create_vcpu, unmapped once on drop.
            unsafe {
                libc::munmap(self.kvm_run.as_ptr().cast::<libc::c_void>(), self.mmap_size);
            }
        }
    }
}
