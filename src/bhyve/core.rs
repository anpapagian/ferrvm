use std::ffi::CStr;
use std::fs::File;
use std::os::fd::IntoRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use super::api::{
    SegDesc, VEC_DEFAULT, VM_ACTIVATE_CPU, VM_ALLOC_MEMSEG, VM_GET_CAPABILITY, VM_GET_REGISTER,
    VM_GET_SEGMENT_DESCRIPTOR, VM_GLA2GPA_NOFAULT, VM_MAX_NAMELEN, VM_MAX_SEG_NAMELEN,
    VM_MMAP_MEMSEG, VM_RESET_CPU, VM_RUN, VM_SET_REGISTER, VM_SET_RUN_STATE,
    VM_SET_SEGMENT_DESCRIPTOR, VMM_CREATE_VM, VMM_CTL_DEV, VMM_CURRENT_INTERFACE_VERSION,
    VMM_DESTROY_VM, VMM_INTERFACE_VERSION, VMM_VM_SUPPORTED, VRK_RESET, VRS_RUN, VmActivateCpu,
    VmCapType, VmCapability, VmCpuMode, VmCreateReq, VmDestroyReq, VmEntry, VmEntryUnion, VmExit,
    VmGla2Gpa, VmGuestPaging, VmMemmap, VmMemseg, VmPagingMode, VmRegName, VmRegister, VmRunState,
    VmSegDesc, VmVcpuReset,
};
use super::exithandler::handle_exit;
use super::ioctl::{ioctl_with_mut_ref, ioctl_with_ref, ioctl_with_val};
use super::irq;
use crate::traits::{self, Hypervisor, Vcpu, Vm};
use ferrvm::printcrln;

pub struct Bhyve {
    fd: File,
}

impl Hypervisor for Bhyve {
    fn create_vm<'a>(&'a self, name: &str) -> traits::Result<Box<dyn Vm + 'a>> {
        self.create_vm(name)
            .map(|vm| Box::new(vm) as Box<dyn Vm>)
            .map_err(std::convert::Into::into)
    }
}

fn create_vm_req(name: &str) -> VmCreateReq {
    let mut req = VmCreateReq {
        name: [0; VM_MAX_NAMELEN],
        flags: 0,
    };

    let bytes = name.as_bytes();
    let len = bytes.len().min(VM_MAX_NAMELEN - 1); // leave room for null terminator
    for (i, &b) in bytes[..len].iter().enumerate() {
        req.name[i] = b as libc::c_char;
    }
    // req.name[len] is already 0 (null terminator) from initialization

    req
}

fn destroy_vm_req(name: &str) -> VmDestroyReq {
    let mut req = VmDestroyReq {
        name: [0; VM_MAX_NAMELEN],
    };

    let bytes = name.as_bytes();
    let len = bytes.len().min(VM_MAX_NAMELEN - 1); // leave room for null terminator
    for (i, &b) in bytes[..len].iter().enumerate() {
        req.name[i] = b as libc::c_char;
    }
    // req.name[len] is already 0 (null terminator) from initialization

    req
}

impl Bhyve {
    pub fn new() -> Result<Self, String> {
        let bhyve = Self::open()?;

        let api_version = bhyve.get_api_version()?;
        printcrln!("[bhyve] Bhyve API version: {api_version}");
        if api_version != VMM_CURRENT_INTERFACE_VERSION {
            return Err(format!(
                "Unsupported Bhyve API version: got {api_version}, expected {VMM_CURRENT_INTERFACE_VERSION}"
            ));
        }
        printcrln!("[bhyve] API version is supported");

        let mut emsg = [0u8; 128];
        let fd = bhyve.fd.as_raw_fd();
        // SAFETY: fd is a valid vmm ctl handle and emsg is a correctly sized buffer.
        let ret = unsafe { ioctl_with_mut_ref(fd, VMM_VM_SUPPORTED as i32, &mut emsg) };
        if ret.is_ok() {
            printcrln!("[bhyve] Bhyve is supported on this hardware");
            Ok(bhyve)
        } else {
            // SAFETY: emsg is a null-terminated buffer written by the ioctl above.
            let error_msg = unsafe { CStr::from_ptr(emsg.as_ptr().cast::<libc::c_char>()) };
            Err(format!(
                "Bhyve is not supported on this hardware: {}",
                error_msg.to_string_lossy()
            ))
        }
    }

    pub fn open() -> Result<Self, String> {
        let fd = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_EXCL)
            .open(VMM_CTL_DEV)
            .map_err(|e| {
                format!(
                    "Unable to open {VMM_CTL_DEV} (check permissions and that bhyve is available): {e}"
                )
            })?;

        Ok(Self { fd })
    }

    pub fn get_api_version(&self) -> Result<i32, String> {
        let fd = self.fd.as_raw_fd();

        // SAFETY: fd is a valid vmm ctl handle and the ioctl takes a value arg.
        let ret = unsafe { ioctl_with_val(fd, VMM_INTERFACE_VERSION as i32, 0) };
        match ret {
            Ok(val) => Ok(val),
            Err(e) => Err(format!("VMM_INTERFACE_VERSION ioctl failed: {e}")),
        }
    }

    pub fn create_vm(&self, name: &str) -> Result<BhyveVm<'_>, String> {
        let fd = self.fd.as_raw_fd();

        let req = create_vm_req(name);
        // SAFETY: fd is a valid vmm ctl handle and the ioctl arg is correctly sized.
        match unsafe { ioctl_with_ref(fd, VMM_CREATE_VM as i32, &req) } {
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // VM already exists, we might want to destroy and recreate or just open it.
                // For now, let's try to destroy it and recreate to be clean.
                self.destroy_vm(name)?;
                // SAFETY: fd is a valid vmm ctl handle and the ioctl arg is correctly sized.
                if let Err(e) = unsafe { ioctl_with_ref(fd, VMM_CREATE_VM as i32, &req) } {
                    return Err(format!("VMM_CREATE_VM ioctl failed after destruction: {e}"));
                }
            }
            Err(e) => return Err(format!("VMM_CREATE_VM ioctl failed: {e}")),
        }

        let vm_dev_path = format!("/dev/vmm/{name}");
        let vm_fd = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&vm_dev_path)
            .map_err(|e| format!("Failed to open {vm_dev_path}: {e}"))?;

        Ok(BhyveVm {
            fd: vm_fd.into_raw_fd(), // move ownership of the fd into BhyveVm in order to manually close that before destroying the VM
            hypervisor: self,
            name: name.to_string(),
            next_segid: AtomicI32::new(0),
        })
    }

    pub fn destroy_vm(&self, name: &str) -> Result<(), String> {
        let fd = self.fd.as_raw_fd();
        let req = destroy_vm_req(name);
        // SAFETY: fd is a valid vmm ctl handle and the ioctl arg is correctly sized.
        match unsafe { ioctl_with_ref(fd, VMM_DESTROY_VM as i32, &req) } {
            Ok(_) => Ok(()),
            Err(e) => Err(format!("VMM_DESTROY_VM ioctl failed: {e}")),
        }
    }
}

pub struct BhyveVm<'a> {
    hypervisor: &'a Bhyve,
    fd: i32,
    name: String,
    next_segid: AtomicI32,
}

impl Drop for BhyveVm<'_> {
    fn drop(&mut self) {
        printcrln!("[bhyve] VM dropped and will be destroyed");

        // SAFETY: self.fd is an owned vm fd closed once on drop.
        unsafe {
            libc::close(self.fd);
        }

        self.hypervisor.destroy_vm(&self.name).unwrap_or_else(|e| {
            printcrln!("[bhyve] Warning: Failed to destroy VM during drop: {e}");
        });

        printcrln!("[bhyve] VM destruction complete");
    }
}

impl Vm for BhyveVm<'_> {
    fn setup(&self) -> traits::Result<()> {
        Ok(())
    }

    fn create_vcpu<'a>(&'a self, id: u32) -> traits::Result<Box<dyn Vcpu + Send + 'a>> {
        printcrln!("[bhyve] Creating vCPU {id}");
        let fd = self.fd.as_raw_fd();

        // Perform initial vCPU setup (activation and reset) immediately upon creation.
        // This ensures the vCPU is in a clean state BEFORE the bootloader sets up registers.
        let ac = VmActivateCpu { vcpuid: id as i32 };
        // SAFETY: fd is a valid VM handle and the ioctl arg is correctly sized.
        unsafe {
            ioctl_with_ref(fd, VM_ACTIVATE_CPU as i32, &ac)
                .map_err(|e| format!("VM_ACTIVATE_CPU ioctl failed: {e}"))?;
        }

        let vvr = VmVcpuReset {
            vcpuid: id as i32,
            kind: VRK_RESET,
        };
        // SAFETY: fd is a valid VM handle and the ioctl arg is correctly sized.
        unsafe {
            ioctl_with_ref(fd, VM_RESET_CPU as i32, &vvr)
                .map_err(|e| format!("VM_RESET_CPU ioctl failed: {e}"))?;
        }

        Ok(Box::new(BhyveVcpu {
            vm: self,
            id,
            next_cmd: VEC_DEFAULT,
            // SAFETY: union of POD variants, all-zero is a valid initial state.
            next_entry_u: unsafe { std::mem::zeroed() },
        }))
    }

    fn register_memory_region(&self, gpa: u64, size: u64, hpa: u64) -> traits::Result<()> {
        let fd = self.fd.as_raw_fd();
        let segid = self.next_segid.fetch_add(1, Ordering::SeqCst);

        let memseg = VmMemseg {
            segid,
            len: size as libc::size_t,
            name: [0; VM_MAX_SEG_NAMELEN],
        };

        // SAFETY: fd is a valid VM handle and the ioctl arg is correctly sized.
        let ret = unsafe { ioctl_with_ref(fd, VM_ALLOC_MEMSEG as i32, &memseg) };
        if let Err(e) = ret {
            return Err(format!("VM_ALLOC_MEMSEG ioctl failed: {e}").into());
        }

        let memmap = VmMemmap {
            gpa,
            segid,
            segoff: 0,
            len: size as libc::size_t,
            prot: libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
            flags: 0,
        };

        // SAFETY: fd is a valid VM handle and the ioctl arg is correctly sized.
        let ret = unsafe { ioctl_with_ref(fd, VM_MMAP_MEMSEG as i32, &memmap) };
        if let Err(e) = ret {
            return Err(format!("VM_MMAP_MEMSEG ioctl failed: {e}").into());
        }

        // SAFETY: fixed mapping of the memseg into the host hpa via the vm fd; result is checked below.
        let ptr = unsafe {
            libc::mmap(
                hpa as *mut libc::c_void,
                size as libc::size_t,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED | libc::MAP_FIXED,
                fd,
                gpa as libc::off_t,
            )
        };

        if ptr == libc::MAP_FAILED {
            let err = std::io::Error::last_os_error();
            return Err(format!("mmap failed for bhyve memory region: {err}").into());
        }

        printcrln!(
            "[bhyve] Registered memory region: gpa=0x{:X}, size=0x{:X}, hpa=0x{:X} (segid={segid})",
            gpa,
            size,
            hpa
        );

        Ok(())
    }

    fn register_irq(&self, irq: u32) -> traits::Result<Arc<dyn traits::IrqSink>> {
        Ok(Arc::new(irq::BhyveIrqSink::new(self.fd, irq)))
    }
}

impl From<&traits::Segment> for SegDesc {
    fn from(seg: &traits::Segment) -> Self {
        let access = (u32::from(seg.type_) & 0x000f)
            | ((u32::from(seg.s) & 0x1) << 4)
            | ((u32::from(seg.dpl) & 0x3) << 5)
            | ((u32::from(seg.present) & 0x1) << 7)
            | ((u32::from(seg.avl) & 0x1) << 12)
            | ((u32::from(seg.l) & 0x1) << 13)
            | ((u32::from(seg.db) & 0x1) << 14)
            | ((u32::from(seg.g) & 0x1) << 15)
            | ((u32::from(seg.unusable) & 0x1) << 16);
        SegDesc {
            base: seg.base,
            limit: seg.limit,
            access,
        }
    }
}

impl From<&SegDesc> for traits::Segment {
    fn from(desc: &SegDesc) -> Self {
        traits::Segment {
            base: desc.base,
            limit: desc.limit,
            selector: 0, // Populated separately
            type_: (desc.access & 0x000f) as u8,
            s: ((desc.access >> 4) & 0x1) as u8,
            dpl: ((desc.access >> 5) & 0x3) as u8,
            present: ((desc.access >> 7) & 0x1) as u8,
            avl: ((desc.access >> 12) & 0x1) as u8,
            l: ((desc.access >> 13) & 0x1) as u8,
            db: ((desc.access >> 14) & 0x1) as u8,
            g: ((desc.access >> 15) & 0x1) as u8,
            unusable: ((desc.access >> 16) & 0x1) as u8,
            padding: 0,
        }
    }
}

impl From<&traits::Dtable> for SegDesc {
    fn from(dt: &traits::Dtable) -> Self {
        SegDesc {
            base: dt.base,
            limit: u32::from(dt.limit),
            access: 0,
        }
    }
}

impl From<&SegDesc> for traits::Dtable {
    fn from(desc: &SegDesc) -> Self {
        traits::Dtable {
            base: desc.base,
            limit: desc.limit as u16,
            padding: [0; 3],
        }
    }
}

pub struct BhyveVcpu<'a> {
    vm: &'a BhyveVm<'a>,
    id: u32,
    next_cmd: u32,
    next_entry_u: VmEntryUnion,
}

impl BhyveVcpu<'_> {
    pub fn gla2gpa(&self, gla: u64, prot: libc::c_int) -> traits::Result<u64> {
        let sregs = self.get_sregs()?;

        let cpu_mode = if (sregs.efer & 0x400) != 0 {
            if sregs.cs.l != 0 {
                VmCpuMode::SixtyFourBit
            } else {
                VmCpuMode::Compatibility
            }
        } else if (sregs.cr0 & 1) != 0 {
            VmCpuMode::Protected
        } else {
            VmCpuMode::Real
        };

        let paging_mode = if (sregs.cr0 & 0x8000_0000) == 0 {
            VmPagingMode::Flat
        } else if (sregs.cr4 & 0x20) == 0 {
            VmPagingMode::ThirtyTwo
        } else if (sregs.efer & 0x100) != 0 {
            VmPagingMode::SixtyFour
        } else {
            VmPagingMode::Pae
        };

        let paging = VmGuestPaging {
            cr3: sregs.cr3,
            cpl: i32::from(sregs.cs.dpl & 3),
            cpu_mode,
            paging_mode,
        };

        let mut gg = VmGla2Gpa {
            vcpuid: self.id as i32,
            prot,
            gla,
            paging,
            fault: 0,
            gpa: 0,
        };

        // SAFETY: vm fd is a valid VM handle and the ioctl arg is correctly sized.
        let ret = unsafe {
            ioctl_with_mut_ref(self.vm.fd.as_raw_fd(), VM_GLA2GPA_NOFAULT as i32, &mut gg)
        };

        if let Err(e) = ret {
            return Err(format!("VM_GLA2GPA_NOFAULT ioctl failed: {e}").into());
        }

        if gg.fault != 0 {
            return Err(format!("GLA2GPA translation fault: {}", gg.fault).into());
        }

        Ok(gg.gpa)
    }

    pub fn read_instr(&self, memory: &crate::memory::GuestMemory) -> traits::Result<Vec<u8>> {
        let regs = self.get_regs()?;
        let rip = regs.rip;

        // Translate GVA to GPA
        let gpa = self.gla2gpa(rip, libc::PROT_READ).map_or(rip, |gpa| gpa);

        memory.read_at(gpa, 15).map_err(std::convert::Into::into)
    }

    fn set_register(&self, reg: VmRegName, val: u64) -> traits::Result<()> {
        let vmreg = VmRegister {
            cpuid: self.id as i32,
            regnum: reg as i32,
            regval: val,
        };
        // SAFETY: vm fd is a valid VM handle and the ioctl arg is correctly sized.
        let ret = unsafe { ioctl_with_ref(self.vm.fd.as_raw_fd(), VM_SET_REGISTER as i32, &vmreg) };
        if let Err(e) = ret {
            return Err(format!("VM_SET_REGISTER ioctl failed for {reg:?}: {e}").into());
        }
        Ok(())
    }

    fn get_register(&self, reg: VmRegName) -> traits::Result<u64> {
        let mut vmreg = VmRegister {
            cpuid: self.id as i32,
            regnum: reg as i32,
            regval: 0,
        };
        // SAFETY: vm fd is a valid VM handle and the ioctl arg is correctly sized.
        let ret = unsafe {
            ioctl_with_mut_ref(self.vm.fd.as_raw_fd(), VM_GET_REGISTER as i32, &mut vmreg)
        };
        if let Err(e) = ret {
            return Err(format!("VM_GET_REGISTER ioctl failed for {reg:?}: {e}").into());
        }
        Ok(vmreg.regval)
    }

    fn set_segment(&self, reg: VmRegName, seg: &traits::Segment) -> traits::Result<()> {
        printcrln!("[bhyve] Setting segment {:?} to: type={} ", reg, seg.type_,);

        let desc: SegDesc = seg.into();
        let vmsegdesc = VmSegDesc {
            cpuid: self.id as i32,
            regnum: reg as i32,
            desc,
        };
        // SAFETY: vm fd is a valid VM handle and the ioctl arg is correctly sized.
        let ret = unsafe {
            ioctl_with_ref(
                self.vm.fd.as_raw_fd(),
                VM_SET_SEGMENT_DESCRIPTOR as i32,
                &vmsegdesc,
            )
        };
        if let Err(e) = ret {
            return Err(format!("VM_SET_SEGMENT_DESCRIPTOR ioctl failed for {reg:?}: {e}").into());
        }
        self.set_register(reg, u64::from(seg.selector))?;
        Ok(())
    }

    fn get_segment(&self, reg: VmRegName) -> traits::Result<traits::Segment> {
        let mut vmsegdesc = VmSegDesc {
            cpuid: self.id as i32,
            regnum: reg as i32,
            desc: SegDesc {
                base: 0,
                limit: 0,
                access: 0,
            },
        };
        // SAFETY: vm fd is a valid VM handle and the ioctl arg is correctly sized.
        let ret = unsafe {
            ioctl_with_mut_ref(
                self.vm.fd.as_raw_fd(),
                VM_GET_SEGMENT_DESCRIPTOR as i32,
                &mut vmsegdesc,
            )
        };
        if let Err(e) = ret {
            return Err(format!("VM_GET_SEGMENT_DESCRIPTOR ioctl failed for {reg:?}: {e}").into());
        }
        let mut seg: traits::Segment = (&vmsegdesc.desc).into();
        seg.selector = self.get_register(reg)? as u16;
        Ok(seg)
    }

    fn set_dtable(&self, reg: VmRegName, dt: &traits::Dtable) -> traits::Result<()> {
        let desc: SegDesc = dt.into();
        let vmsegdesc = VmSegDesc {
            cpuid: self.id as i32,
            regnum: reg as i32,
            desc,
        };
        // SAFETY: vm fd is a valid VM handle and the ioctl arg is correctly sized.
        let ret = unsafe {
            ioctl_with_ref(
                self.vm.fd.as_raw_fd(),
                VM_SET_SEGMENT_DESCRIPTOR as i32,
                &vmsegdesc,
            )
        };
        if let Err(e) = ret {
            return Err(format!("VM_SET_SEGMENT_DESCRIPTOR ioctl failed for {reg:?}: {e}").into());
        }
        Ok(())
    }

    fn get_dtable(&self, reg: VmRegName) -> traits::Result<traits::Dtable> {
        let mut vmsegdesc = VmSegDesc {
            cpuid: self.id as i32,
            regnum: reg as i32,
            desc: SegDesc {
                base: 0,
                limit: 0,
                access: 0,
            },
        };
        // SAFETY: vm fd is a valid VM handle and the ioctl arg is correctly sized.
        let ret = unsafe {
            ioctl_with_mut_ref(
                self.vm.fd.as_raw_fd(),
                VM_GET_SEGMENT_DESCRIPTOR as i32,
                &mut vmsegdesc,
            )
        };
        if let Err(e) = ret {
            return Err(format!("VM_GET_SEGMENT_DESCRIPTOR ioctl failed for {reg:?}: {e}").into());
        }
        Ok((&vmsegdesc.desc).into())
    }

    pub fn run(&self) -> Result<VmExit, String> {
        // SAFETY: POD struct, all-zero is a valid initial state.
        let mut vm_exit: VmExit = unsafe { std::mem::zeroed() };
        let vm_entry = VmEntry {
            cpuid: self.id as i32,
            cmd: self.next_cmd,
            exit_data: &raw mut vm_exit,
            u: self.next_entry_u,
        };

        let fd = self.vm.fd.as_raw_fd();
        // SAFETY: fd is a valid VM handle and vm_entry holds a valid exit_data pointer.
        let ret = unsafe { ioctl_with_ref(fd, VM_RUN as i32, &vm_entry) };

        match ret {
            Ok(_) => Ok(vm_exit),
            Err(e) => Err(format!("VM_RUN ioctl failed: {e}")),
        }
    }

    fn get_capability(&self, cap: VmCapType) -> traits::Result<libc::c_int> {
        let mut vmcap = VmCapability {
            cpuid: self.id as i32,
            captype: cap,
            capval: 0,
            allcpus: 0,
        };
        // SAFETY: vm fd is a valid VM handle and the ioctl arg is correctly sized.
        let ret = unsafe {
            ioctl_with_mut_ref(self.vm.fd.as_raw_fd(), VM_GET_CAPABILITY as i32, &mut vmcap)
        };
        if let Err(e) = ret {
            return Err(format!("VM_GET_CAPABILITY ioctl failed for {cap:?}: {e}").into());
        }
        Ok(vmcap.capval)
    }
}

impl Vcpu for BhyveVcpu<'_> {
    fn setup(&self) -> traits::Result<()> {
        let fd = self.vm.fd.as_raw_fd();

        let capabilities = [
            (VmCapType::HaltExit, "HaltExit"),
            (VmCapType::MtrapExit, "MtrapExit"),
            (VmCapType::PauseExit, "PauseExit"),
            (VmCapType::EnableInvpcid, "EnableInvpcid"),
            (VmCapType::BptExit, "BptExit"),
        ];

        for (cap_type, name) in capabilities {
            let cap = self.get_capability(cap_type)?;
            match cap {
                0 => printcrln!("[bhyve] {} capability is not supported on this vCPU", name),
                1 => printcrln!("[bhyve] {} capability is supported on this vCPU", name),
                _ => printcrln!(
                    "[bhyve] {} capability returned unexpected value: {}",
                    name,
                    cap
                ),
            }
        }

        printcrln!("[bhyve] Setting vCPU {} run state to RUN", self.id);
        let vrs = VmRunState {
            vcpuid: self.id as i32,
            state: VRS_RUN,
            sipi_vector: 0,
            _pad: [0; 3],
        };
        // SAFETY: fd is a valid VM handle and the ioctl arg is correctly sized.
        let ret = unsafe { ioctl_with_ref(fd, VM_SET_RUN_STATE as i32, &vrs) };
        if let Err(e) = ret {
            return Err(format!("VM_SET_RUN_STATE ioctl failed: {e}").into());
        }

        Ok(())
    }

    fn step_run(
        &mut self,
        stats: &mut crate::stats::ExitStats,
        memory: &crate::memory::GuestMemory,
        pio_bus: &crate::bus::Bus,
        mmio_bus: &crate::bus::Bus,
        config: &crate::config::Config,
    ) -> core::result::Result<bool, std::io::Error> {
        match self.run() {
            Ok(vm_exit) => {
                let (continue_run, next_cmd, next_u) =
                    handle_exit(&vm_exit, stats, self, memory, pio_bus, mmio_bus, config);
                self.next_cmd = next_cmd;
                self.next_entry_u = next_u;
                Ok(continue_run)
            }
            Err(e) => Err(std::io::Error::other(e)),
        }
    }

    fn set_regs(&self, regs: &traits::VcpuRegs) -> traits::Result<()> {
        self.set_register(VmRegName::Rax, regs.rax)?;
        self.set_register(VmRegName::Rbx, regs.rbx)?;
        self.set_register(VmRegName::Rcx, regs.rcx)?;
        self.set_register(VmRegName::Rdx, regs.rdx)?;
        self.set_register(VmRegName::Rsi, regs.rsi)?;
        self.set_register(VmRegName::Rdi, regs.rdi)?;
        self.set_register(VmRegName::Rsp, regs.rsp)?;
        self.set_register(VmRegName::Rbp, regs.rbp)?;
        self.set_register(VmRegName::R8, regs.r8)?;
        self.set_register(VmRegName::R9, regs.r9)?;
        self.set_register(VmRegName::R10, regs.r10)?;
        self.set_register(VmRegName::R11, regs.r11)?;
        self.set_register(VmRegName::R12, regs.r12)?;
        self.set_register(VmRegName::R13, regs.r13)?;
        self.set_register(VmRegName::R14, regs.r14)?;
        self.set_register(VmRegName::R15, regs.r15)?;
        self.set_register(VmRegName::Rip, regs.rip)?;
        self.set_register(VmRegName::Rflags, regs.rflags)?;
        Ok(())
    }

    fn get_regs(&self) -> traits::Result<traits::VcpuRegs> {
        Ok(traits::VcpuRegs {
            rax: self.get_register(VmRegName::Rax)?,
            rbx: self.get_register(VmRegName::Rbx)?,
            rcx: self.get_register(VmRegName::Rcx)?,
            rdx: self.get_register(VmRegName::Rdx)?,
            rsi: self.get_register(VmRegName::Rsi)?,
            rdi: self.get_register(VmRegName::Rdi)?,
            rsp: self.get_register(VmRegName::Rsp)?,
            rbp: self.get_register(VmRegName::Rbp)?,
            r8: self.get_register(VmRegName::R8)?,
            r9: self.get_register(VmRegName::R9)?,
            r10: self.get_register(VmRegName::R10)?,
            r11: self.get_register(VmRegName::R11)?,
            r12: self.get_register(VmRegName::R12)?,
            r13: self.get_register(VmRegName::R13)?,
            r14: self.get_register(VmRegName::R14)?,
            r15: self.get_register(VmRegName::R15)?,
            rip: self.get_register(VmRegName::Rip)?,
            rflags: self.get_register(VmRegName::Rflags)?,
        })
    }

    fn set_sregs(&self, sregs: &traits::VcpuSregs) -> traits::Result<()> {
        self.set_segment(VmRegName::Cs, &sregs.cs)?;
        self.set_segment(VmRegName::Ds, &sregs.ds)?;
        self.set_segment(VmRegName::Es, &sregs.es)?;
        self.set_segment(VmRegName::Fs, &sregs.fs)?;
        self.set_segment(VmRegName::Gs, &sregs.gs)?;
        self.set_segment(VmRegName::Ss, &sregs.ss)?;
        self.set_segment(VmRegName::Tr, &sregs.tr)?;
        self.set_segment(VmRegName::Ldtr, &sregs.ldt)?;
        self.set_dtable(VmRegName::Gdtr, &sregs.gdt)?;
        self.set_dtable(VmRegName::Idtr, &sregs.idt)?;
        self.set_register(VmRegName::Cr0, sregs.cr0)?;
        self.set_register(VmRegName::Cr2, sregs.cr2)?;
        self.set_register(VmRegName::Cr3, sregs.cr3)?;
        self.set_register(VmRegName::Cr4, sregs.cr4)?;
        self.set_register(VmRegName::Efer, sregs.efer)?;
        Ok(())
    }

    fn get_sregs(&self) -> traits::Result<traits::VcpuSregs> {
        Ok(traits::VcpuSregs {
            cs: self.get_segment(VmRegName::Cs)?,
            ds: self.get_segment(VmRegName::Ds)?,
            es: self.get_segment(VmRegName::Es)?,
            fs: self.get_segment(VmRegName::Fs)?,
            gs: self.get_segment(VmRegName::Gs)?,
            ss: self.get_segment(VmRegName::Ss)?,
            tr: self.get_segment(VmRegName::Tr)?,
            ldt: self.get_segment(VmRegName::Ldtr)?,
            gdt: self.get_dtable(VmRegName::Gdtr)?,
            idt: self.get_dtable(VmRegName::Idtr)?,
            cr0: self.get_register(VmRegName::Cr0)?,
            cr2: self.get_register(VmRegName::Cr2)?,
            cr3: self.get_register(VmRegName::Cr3)?,
            cr4: self.get_register(VmRegName::Cr4)?,
            cr8: 0,
            efer: self.get_register(VmRegName::Efer)?,
            apic_base: 0,
            interrupt_bitmap: [0; 4],
        })
    }
}
