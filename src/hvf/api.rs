//! Minimal bindings for Hypervisor.framework on Apple silicon.

use std::ffi::c_void;

pub type HvReturn = i32;
pub type HvVcpu = u64;
pub type HvReg = u32;
pub type HvSysReg = u16;
pub type HvGicConfig = *mut c_void;

pub const HV_SUCCESS: HvReturn = 0;

pub const HV_MEMORY_READ: u64 = 1 << 0;
pub const HV_MEMORY_WRITE: u64 = 1 << 1;
pub const HV_MEMORY_EXEC: u64 = 1 << 2;

pub const HV_EXIT_REASON_CANCELED: u32 = 0;
pub const HV_EXIT_REASON_EXCEPTION: u32 = 1;

pub const HV_REG_X0: HvReg = 0;
pub const HV_REG_PC: HvReg = 31;
pub const HV_REG_CPSR: HvReg = 34;

pub const HV_SYS_REG_MPIDR_EL1: HvSysReg = 0xc005;

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct HvVcpuExitException {
    pub syndrome: u64,
    pub virtual_address: u64,
    pub physical_address: u64,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct HvVcpuExit {
    pub reason: u32,
    pub exception: HvVcpuExitException,
}

#[link(name = "Hypervisor", kind = "framework")]
unsafe extern "C" {
    pub fn hv_vm_create(config: *mut c_void) -> HvReturn;
    pub fn hv_vm_destroy() -> HvReturn;
    pub fn hv_vm_map(addr: *mut c_void, ipa: u64, size: usize, flags: u64) -> HvReturn;

    pub fn hv_vcpu_create(
        vcpu: *mut HvVcpu,
        exit: *mut *mut HvVcpuExit,
        config: *mut c_void,
    ) -> HvReturn;
    pub fn hv_vcpu_destroy(vcpu: HvVcpu) -> HvReturn;
    pub fn hv_vcpu_run(vcpu: HvVcpu) -> HvReturn;
    pub fn hv_vcpu_get_reg(vcpu: HvVcpu, reg: HvReg, value: *mut u64) -> HvReturn;
    pub fn hv_vcpu_set_reg(vcpu: HvVcpu, reg: HvReg, value: u64) -> HvReturn;
    pub fn hv_vcpu_set_sys_reg(vcpu: HvVcpu, reg: HvSysReg, value: u64) -> HvReturn;

    pub fn hv_gic_config_create() -> HvGicConfig;
    pub fn hv_gic_config_set_distributor_base(config: HvGicConfig, base: u64) -> HvReturn;
    pub fn hv_gic_config_set_redistributor_base(config: HvGicConfig, base: u64) -> HvReturn;
    pub fn hv_gic_create(config: HvGicConfig) -> HvReturn;
    pub fn hv_gic_get_distributor_size(size: *mut usize) -> HvReturn;
    pub fn hv_gic_get_redistributor_region_size(size: *mut usize) -> HvReturn;
    pub fn hv_gic_set_spi(intid: u32, level: bool) -> HvReturn;
}

pub fn result(value: HvReturn, operation: &str) -> crate::traits::Result<()> {
    if value == HV_SUCCESS {
        Ok(())
    } else {
        Err(format!("{operation} failed with Hypervisor error {value:#x}").into())
    }
}
