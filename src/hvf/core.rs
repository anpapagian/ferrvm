use std::ffi::c_void;
use std::io;
use std::ptr::NonNull;
use std::sync::Arc;

use ferrvm::printcrln;

use super::{api, irq};
use crate::machine::{GIC_DISTRIBUTOR_BASE, GIC_REDISTRIBUTOR_BASE, GIC_REDISTRIBUTOR_SIZE};
use crate::traits::{self, Hypervisor, Vcpu, Vm};

pub struct Hvf;

impl Hvf {
    pub const fn new() -> Self {
        Self
    }
}

impl Hypervisor for Hvf {
    fn create_vm<'a>(&'a self, _name: &str) -> traits::Result<Box<dyn Vm + 'a>> {
        api::result(
            // SAFETY: A null configuration requests Hypervisor.framework defaults.
            unsafe { api::hv_vm_create(std::ptr::null_mut()) },
            "hv_vm_create",
        )?;
        Ok(Box::new(HvfVm))
    }
}

pub struct HvfVm;

impl Vm for HvfVm {
    fn setup(&self) -> traits::Result<()> {
        // SAFETY: Configuration creation has no preconditions after VM creation.
        let config = unsafe { api::hv_gic_config_create() };
        if config.is_null() {
            return Err("hv_gic_config_create returned null".into());
        }

        api::result(
            // SAFETY: The fixed address is aligned and does not overlap guest RAM.
            unsafe { api::hv_gic_config_set_distributor_base(config, GIC_DISTRIBUTOR_BASE) },
            "hv_gic_config_set_distributor_base",
        )?;
        api::result(
            // SAFETY: The fixed address is aligned and does not overlap guest RAM.
            unsafe { api::hv_gic_config_set_redistributor_base(config, GIC_REDISTRIBUTOR_BASE) },
            "hv_gic_config_set_redistributor_base",
        )?;
        api::result(
            // SAFETY: config was created above and remains valid for this call.
            unsafe { api::hv_gic_create(config) },
            "hv_gic_create",
        )?;

        let (mut dist_size, mut redist_size) = (0usize, 0usize);
        api::result(
            // SAFETY: The output pointer refers to initialized, writable storage.
            unsafe { api::hv_gic_get_distributor_size(&raw mut dist_size) },
            "hv_gic_get_distributor_size",
        )?;
        api::result(
            // SAFETY: The output pointer refers to initialized, writable storage.
            unsafe { api::hv_gic_get_redistributor_region_size(&raw mut redist_size) },
            "hv_gic_get_redistributor_region_size",
        )?;
        if redist_size as u64 > GIC_REDISTRIBUTOR_SIZE {
            return Err(
                format!("GIC redistributor region is too large: {redist_size:#x} bytes").into(),
            );
        }
        printcrln!(
            "[hvf] GICv3: distributor {:#x}+{dist_size:#x}, redistributors {:#x}+{redist_size:#x}",
            GIC_DISTRIBUTOR_BASE,
            GIC_REDISTRIBUTOR_BASE
        );
        Ok(())
    }

    fn create_vcpu<'a>(&'a self, id: u32) -> traits::Result<Box<dyn Vcpu + 'a>> {
        if id != 0 {
            return Err("the arm64 machine currently supports one vCPU".into());
        }
        let mut handle = 0;
        let mut exit = std::ptr::null_mut();
        api::result(
            // SAFETY: Both output pointers are valid and the GIC was created first.
            unsafe { api::hv_vcpu_create(&raw mut handle, &raw mut exit, std::ptr::null_mut()) },
            "hv_vcpu_create",
        )?;
        let exit = NonNull::new(exit).ok_or("hv_vcpu_create returned a null exit pointer")?;
        Ok(Box::new(HvfVcpu { handle, exit }))
    }

    fn register_memory_region(&self, gpa: u64, size: u64, hpa: u64) -> traits::Result<()> {
        let size = usize::try_from(size).map_err(|_| "guest memory is too large")?;
        api::result(
            // SAFETY: GuestMemory owns a page-aligned mmap covering size bytes.
            unsafe {
                api::hv_vm_map(
                    hpa as *mut c_void,
                    gpa,
                    size,
                    api::HV_MEMORY_READ | api::HV_MEMORY_WRITE | api::HV_MEMORY_EXEC,
                )
            },
            "hv_vm_map",
        )
    }

    fn register_irq(&self, irq: u32) -> traits::Result<Arc<dyn traits::IrqSink>> {
        if irq < 32 {
            return Err(format!("GIC interrupt {irq} is not an SPI").into());
        }
        Ok(Arc::new(irq::HvfIrqSink::new(irq)))
    }
}

impl Drop for HvfVm {
    fn drop(&mut self) {
        // SAFETY: HvfVm represents the process-global VM and all borrowed vCPUs
        // have been dropped before this value can be dropped.
        let rc = unsafe { api::hv_vm_destroy() };
        if rc != api::HV_SUCCESS {
            eprintln!("[hvf] hv_vm_destroy failed: {rc:#x}");
        }
    }
}

pub struct HvfVcpu {
    handle: api::HvVcpu,
    exit: NonNull<api::HvVcpuExit>,
}

impl HvfVcpu {
    fn get_reg(&self, reg: api::HvReg) -> traits::Result<u64> {
        let mut value = 0;
        api::result(
            // SAFETY: value is writable and this method runs on the owning thread.
            unsafe { api::hv_vcpu_get_reg(self.handle, reg, &raw mut value) },
            "hv_vcpu_get_reg",
        )?;
        Ok(value)
    }

    fn set_reg(&self, reg: api::HvReg, value: u64) -> traits::Result<()> {
        api::result(
            // SAFETY: This vCPU handle is valid and used by its owning thread.
            unsafe { api::hv_vcpu_set_reg(self.handle, reg, value) },
            "hv_vcpu_set_reg",
        )
    }

    fn advance_pc(&self) -> traits::Result<()> {
        self.set_reg(api::HV_REG_PC, self.get_reg(api::HV_REG_PC)? + 4)
    }

    fn handle_exception(
        &self,
        exception: api::HvVcpuExitException,
        stats: &mut crate::stats::ExitStats,
        mmio_bus: &crate::bus::Bus,
    ) -> traits::Result<bool> {
        let syndrome = exception.syndrome;
        let ec = (syndrome >> 26) & 0x3f;
        match ec {
            0x01 => {
                // WFI/WFE
                self.advance_pc()?;
                Ok(true)
            }
            // HVC exits already point past the instruction; SMC exits do not.
            0x16 => self.handle_psci(),
            0x17 => {
                let result = self.handle_psci()?;
                self.advance_pc()?;
                Ok(result)
            }
            0x18 => self.handle_system_instruction(syndrome),
            0x24 => {
                stats.mmio_exits += 1;
                self.handle_mmio(syndrome, exception.physical_address, mmio_bus)?;
                Ok(true)
            }
            0x3c => {
                stats.shutdown_exits += 1;
                Ok(false)
            }
            _ => Err(format!(
                "unhandled ARM exception EC={ec:#x}, ESR={syndrome:#x}, IPA={:#x}, VA={:#x}",
                exception.physical_address, exception.virtual_address
            )
            .into()),
        }
    }

    fn handle_system_instruction(&self, syndrome: u64) -> traits::Result<bool> {
        let op0 = (syndrome >> 20) & 0x3;
        let op1 = (syndrome >> 14) & 0x7;
        let crn = (syndrome >> 10) & 0xf;
        let crm = (syndrome >> 1) & 0xf;
        let op2 = (syndrome >> 17) & 0x7;
        let is_read = syndrome & 1 != 0;

        // Linux clears the OS lock debug registers while setting up its debug
        // architecture. Apple traps these otherwise harmless writes.
        if op0 == 2 && op1 == 0 && crn == 1 && matches!(crm, 0 | 3) && op2 == 4 && !is_read {
            self.advance_pc()?;
            return Ok(true);
        }

        Err(format!(
            "unhandled system instruction at PC={:#x}: ESR={syndrome:#x}, S{op0}_{op1}_C{crn}_C{crm}_{op2}, read={is_read}",
            self.get_reg(api::HV_REG_PC)?
        )
        .into())
    }

    fn handle_psci(&self) -> traits::Result<bool> {
        let function = self.get_reg(api::HV_REG_X0)? as u32;
        match function {
            0x8400_0008 | 0x8400_0009 => Ok(false), // SYSTEM_OFF / SYSTEM_RESET
            0x8400_0000 => {
                self.set_reg(api::HV_REG_X0, 0x0001_0001)?; // PSCI 1.1
                Ok(true)
            }
            0x8400_0006 => {
                self.set_reg(api::HV_REG_X0, 2)?; // MIGRATE_INFO_TYPE: no trusted OS
                Ok(true)
            }
            0x8400_000a => {
                let queried = self.get_reg(api::HV_REG_X0 + 1)? as u32;
                let supported = matches!(
                    queried,
                    0x8400_0000 | 0x8400_0006 | 0x8400_0008 | 0x8400_0009 | 0x8400_000a
                );
                self.set_reg(api::HV_REG_X0, if supported { 0 } else { u64::MAX })?;
                Ok(true)
            }
            _ => {
                self.set_reg(api::HV_REG_X0, u64::MAX)?; // PSCI_NOT_SUPPORTED
                Ok(true)
            }
        }
    }

    fn handle_mmio(
        &self,
        syndrome: u64,
        address: u64,
        mmio_bus: &crate::bus::Bus,
    ) -> traits::Result<()> {
        if syndrome & (1 << 24) == 0 {
            return Err(format!("MMIO data abort has no valid ISS: ESR={syndrome:#x}").into());
        }
        let size = 1_usize << ((syndrome >> 22) & 0x3);
        let reg = ((syndrome >> 16) & 0x1f) as u32;
        let is_64_bit = syndrome & (1 << 15) != 0;
        let sign_extend = syndrome & (1 << 21) != 0;
        let is_write = syndrome & (1 << 6) != 0;
        let mut data = [0_u8; 8];

        if is_write {
            let value = if reg == 31 {
                0
            } else {
                self.get_reg(api::HV_REG_X0 + reg)?
            };
            data[..size].copy_from_slice(&value.to_le_bytes()[..size]);
            if !mmio_bus.write(address, &data[..size]) {
                return Err(format!("unhandled MMIO write at {address:#x}").into());
            }
        } else {
            if !mmio_bus.read(address, &mut data[..size]) {
                return Err(format!("unhandled MMIO read at {address:#x}").into());
            }
            if reg != 31 {
                let mut value = u64::from_le_bytes(data);
                if sign_extend && size < 8 {
                    let shift = 64 - size * 8;
                    value = ((value << shift) as i64 >> shift) as u64;
                }
                if !is_64_bit {
                    value &= u64::from(u32::MAX);
                }
                self.set_reg(api::HV_REG_X0 + reg, value)?;
            }
        }
        self.advance_pc()
    }
}

impl Vcpu for HvfVcpu {
    fn setup(&self) -> traits::Result<()> {
        // SAFETY: Affinity must be assigned before the first vCPU run.
        api::result(
            unsafe { api::hv_vcpu_set_sys_reg(self.handle, api::HV_SYS_REG_MPIDR_EL1, 0) },
            "set MPIDR_EL1",
        )
    }

    fn step_run(
        &mut self,
        stats: &mut crate::stats::ExitStats,
        _memory: &crate::memory::GuestMemory,
        _pio_bus: &crate::bus::Bus,
        mmio_bus: &crate::bus::Bus,
        _config: &crate::config::Config,
    ) -> Result<bool, io::Error> {
        // The native GIC handles virtual timers, including delivery and EOI.
        // SAFETY: Run is called from the thread that created the vCPU.
        api::result(unsafe { api::hv_vcpu_run(self.handle) }, "hv_vcpu_run")
            .map_err(io::Error::other)?;
        // SAFETY: Hypervisor.framework owns this pointer until vCPU destruction.
        let exit = unsafe { *self.exit.as_ptr() };
        match exit.reason {
            api::HV_EXIT_REASON_CANCELED => Err(io::Error::from(io::ErrorKind::Interrupted)),
            api::HV_EXIT_REASON_EXCEPTION => self
                .handle_exception(exit.exception, stats, mmio_bus)
                .map_err(io::Error::other),
            reason => Err(io::Error::other(format!(
                "unhandled Hypervisor exit reason {reason}"
            ))),
        }
    }

    fn set_boot_state(&self, pc: u64, fdt_address: u64) -> traits::Result<()> {
        self.set_reg(api::HV_REG_X0, fdt_address)?;
        self.set_reg(api::HV_REG_X0 + 1, 0)?;
        self.set_reg(api::HV_REG_X0 + 2, 0)?;
        self.set_reg(api::HV_REG_X0 + 3, 0)?;
        self.set_reg(api::HV_REG_PC, pc)?;
        // EL1h with Debug, SError, IRQ and FIQ masked.
        self.set_reg(api::HV_REG_CPSR, 0x3c5)
    }

    fn arm64_state(&self) -> traits::Result<traits::Arm64State> {
        let mut state = traits::Arm64State::default();
        for (index, value) in state.x.iter_mut().enumerate() {
            *value = self.get_reg(api::HV_REG_X0 + index as u32)?;
        }
        state.pc = self.get_reg(api::HV_REG_PC)?;
        state.cpsr = self.get_reg(api::HV_REG_CPSR)?;
        Ok(state)
    }
}

impl Drop for HvfVcpu {
    fn drop(&mut self) {
        // SAFETY: The handle belongs to this object and is destroyed once.
        let rc = unsafe { api::hv_vcpu_destroy(self.handle) };
        if rc != api::HV_SUCCESS {
            eprintln!("[hvf] hv_vcpu_destroy failed: {rc:#x}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{Bus, BusDevice};
    use crate::config::Config;
    use crate::memory::GuestMemory;
    use crate::stats::ExitStats;
    use clap::Parser;
    use std::sync::Mutex;

    struct SignedBytes;

    impl BusDevice for SignedBytes {
        fn read(&mut self, _: u64, data: &mut [u8]) {
            data.fill(0x80);
        }

        fn write(&mut self, _: u64, _: &[u8]) {}
    }

    // Hypervisor.framework permits one VM per process. Keep all guest probes
    // in one test so the test runner never creates competing VM instances.
    #[test]
    fn guest_mmio_loads_and_smccc_calls() {
        use crate::machine::{RAM_BASE, UART_BASE};

        let hypervisor = Hvf::new();
        let vm = hypervisor.create_vm("test").unwrap();
        vm.setup().unwrap();
        let memory = GuestMemory::allocate_at(RAM_BASE, 16384).unwrap();
        vm.register_memory_region(memory.base(), memory.size(), memory.host_addr())
            .unwrap();
        let mut cpu = vm.create_vcpu(0).unwrap();
        cpu.setup().unwrap();
        let config = Config::parse_from(["test", "--kernel", "unused"]);
        let pio = Bus::new();
        let mmio = Bus::new();
        mmio.register(UART_BASE, 4096, Arc::new(Mutex::new(SignedBytes)))
            .unwrap();
        let mut exit_stats = ExitStats::new();

        // Signed and unsigned loads from an emulated device. Writes to W1
        // must clear X1's upper half, even when the loaded value is negative.
        let loads: [(u32, u64); 6] = [
            (0x39c0_0001, 0x0000_0000_ffff_ff80), // ldrsb w1, [x0]
            (0x3980_0001, 0xffff_ffff_ffff_ff80), // ldrsb x1, [x0]
            (0x3940_0001, 0x80),                  // ldrb w1, [x0]
            (0x79c0_0001, 0x0000_0000_ffff_8080), // ldrsh w1, [x0]
            (0x7980_0001, 0xffff_ffff_ffff_8080), // ldrsh x1, [x0]
            (0xb980_0001, 0xffff_ffff_8080_8080), // ldrsw x1, [x0]
        ];
        let calls = [0xd400_0002_u32, 0xd400_0003];
        // Populate the whole program before execution: the guest can cache
        // nearby instructions, so later host writes would require I-cache maintenance.
        for (index, (instruction, _)) in loads.iter().enumerate() {
            memory
                .write_at(RAM_BASE + index as u64 * 4, &instruction.to_le_bytes())
                .unwrap();
        }
        for (index, instruction) in calls.iter().enumerate() {
            memory
                .write_at(
                    RAM_BASE + 0x100 + index as u64 * 4,
                    &instruction.to_le_bytes(),
                )
                .unwrap();
        }
        for (index, (instruction, expected)) in loads.into_iter().enumerate() {
            let pc = RAM_BASE + index as u64 * 4;
            cpu.set_boot_state(pc, UART_BASE).unwrap();
            assert!(
                cpu.step_run(&mut exit_stats, &memory, &pio, &mmio, &config)
                    .unwrap()
            );
            let state = cpu.arm64_state().unwrap();
            assert_eq!(state.x[1], expected, "load instruction {instruction:#x}");
            assert_eq!(state.pc, pc + 4);
        }

        // Unsupported calls return -1 and resume after both HVC and SMC.
        // HVC has already advanced the saved PC when the host receives it.
        for (index, instruction) in calls.into_iter().enumerate() {
            let pc = RAM_BASE + 0x100 + index as u64 * 4;
            cpu.set_boot_state(pc, 0xdead_beef).unwrap();
            assert!(
                cpu.step_run(&mut exit_stats, &memory, &pio, &mmio, &config)
                    .unwrap()
            );
            let state = cpu.arm64_state().unwrap();
            assert_eq!(state.x[0], u64::MAX);
            assert_eq!(state.pc, pc + 4, "SMCCC instruction {instruction:#x}");
        }
    }
}
