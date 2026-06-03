use super::api::{
    INOUT_IN, VEC_DEFAULT, VEC_FULFILL_INOUT, VEC_FULFILL_MMIO, VmEntryUnion, VmExit, VmExitCode,
    VmInout,
};
use super::core::BhyveVcpu;
use crate::bus::Bus;
use crate::config::Config;
use crate::hypervisor::hypervisor_backend::api;
use crate::stats::ExitStats;
use crate::traits::Vcpu;
use ferrvm::printcrln;
use iced_x86::{Decoder, DecoderOptions, Formatter, IntelFormatter};

#[allow(clippy::too_many_lines)]
pub fn handle_exit(
    vm_exit: &VmExit,
    stats: &mut ExitStats,
    vcpu: &BhyveVcpu,
    memory: &crate::memory::GuestMemory,
    pio_bus: &Bus,
    mmio_bus: &Bus,
    config: &Config,
) -> (bool, u32, api::VmEntryUnion) {
    let mut next_cmd = VEC_DEFAULT;
    // SAFETY: POD union, all-zero is a valid initial state.
    let mut next_u: VmEntryUnion = unsafe { std::mem::zeroed() };

    match vm_exit.exitcode {
        VmExitCode::Inout => {
            // SAFETY: inout variant is valid for an inout exit.
            let mut inout = unsafe { vm_exit.u.inout };
            handle_io_exit(&mut inout, stats, pio_bus);

            next_cmd = VEC_FULFILL_INOUT;
            next_u.inout = inout;
            (true, next_cmd, next_u)
        }
        VmExitCode::Mmio => {
            // SAFETY: mmio variant is valid for an mmio exit.
            let mut mmio = unsafe { vm_exit.u.mmio };
            handle_mmio_exit(&mut mmio, stats, mmio_bus);

            next_cmd = VEC_FULFILL_MMIO;
            next_u.mmio = mmio;
            (true, next_cmd, next_u)
        }
        VmExitCode::Hlt => {
            stats.hlt_exits += 1;
            (true, next_cmd, next_u)
        }
        VmExitCode::Suspended => {
            // SAFETY: suspended variant is valid for a suspended exit.
            let suspended = unsafe { vm_exit.u.suspended };
            printcrln!(
                "[exit] Guest suspended (how: {:?}, source: {})",
                suspended.how,
                suspended.source
            );
            stats.shutdown_exits += 1;
            (false, next_cmd, next_u)
        }
        VmExitCode::Vmx => {
            // SAFETY: vmx variant is valid for a vmx exit.
            let vmx = unsafe { vm_exit.u.vmx };
            printcrln!(
                "[exit] VMX exit: status {}, reason {}, qualification 0x{:X}, RIP 0x{:X}",
                vmx.status,
                vmx.exit_reason,
                vmx.exit_qualification,
                vm_exit.rip
            );
            stats.other_exits += 1;
            (true, next_cmd, next_u)
        }
        VmExitCode::Rdmsr => {
            // SAFETY: msr variant is valid for an rdmsr exit.
            let msr = unsafe { vm_exit.u.msr };
            printcrln!(
                "[exit] RDMSR exit: msr 0x{:X} at RIP 0x{:X}",
                msr.code,
                vm_exit.rip
            );
            (true, next_cmd, next_u)
        }
        VmExitCode::Wrmsr => {
            // SAFETY: msr variant is valid for a wrmsr exit.
            let msr = unsafe { vm_exit.u.msr };
            let regs = vcpu.get_regs().unwrap_or_default();
            let rax = regs.rax & 0xFFFF_FFFF;
            let rdx = regs.rdx & 0xFFFF_FFFF;
            let val = (rdx << 32) | rax;

            printcrln!(
                "[exit] WRMSR exit: msr 0x{:X} val 0x{:X} at RIP 0x{:X}",
                msr.code,
                val,
                vm_exit.rip
            );
            (true, next_cmd, next_u)
        }
        VmExitCode::Bogus => {
            stats.other_exits += 1;

            // In debug mode, dump information about the bogus exit.
            if !config.debug {
                return (true, next_cmd, next_u);
            }

            let Ok(regs) = vcpu.get_regs() else {
                printcrln!("[exit] Failed to get registers for BOGUS exit");
                return (true, next_cmd, next_u);
            };

            let Ok(sregs) = vcpu.get_sregs() else {
                printcrln!("[exit] Failed to get special registers for BOGUS exit");
                return (true, next_cmd, next_u);
            };

            printcrln!(
                "BOGUS exit at RIP={:#x} CR0={:#x} CR4={:#x} vm_exit.inst_length: {} vm_exit.rip={:#x}",
                regs.rip,
                sregs.cr0,
                sregs.cr4,
                vm_exit.inst_length,
                vm_exit.rip,
            );

            match vcpu.read_instr(memory) {
                Ok(instr) => {
                    let len = if vm_exit.inst_length > 0 {
                        vm_exit.inst_length as usize
                    } else {
                        15
                    };
                    let instr_bytes = &instr[..len.min(instr.len())];
                    let instr_hex = instr_bytes
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<Vec<String>>()
                        .join(" ");

                    let bitness = if (sregs.efer & 0x400) != 0 {
                        if sregs.cs.l != 0 { 64 } else { 32 }
                    } else if (sregs.cr0 & 1) != 0 {
                        32
                    } else {
                        16
                    };

                    let mut decoder =
                        Decoder::with_ip(bitness, instr_bytes, regs.rip, DecoderOptions::NONE);
                    let mut output = String::new();
                    let mut formatter = IntelFormatter::new();
                    if let Some(instruction) = decoder.iter().next() {
                        formatter.format(&instruction, &mut output);
                        printcrln!("[exit] Instruction: {}", output);
                    } else {
                        printcrln!("[exit] Instruction bytes: {}", instr_hex);
                    }
                }
                Err(e) => {
                    printcrln!("[exit] Failed to read instruction: {}", e);
                }
            }

            (true, next_cmd, next_u)
        }
        _ => {
            printcrln!(
                "[exit] Unhandled exit: {:?} (code: {}) at RIP 0x{:X} (inst_len: {})",
                vm_exit.exitcode,
                vm_exit.exitcode as u32,
                vm_exit.rip,
                vm_exit.inst_length
            );

            stats.other_exits += 1;
            (true, next_cmd, next_u)
        }
    }
}

fn handle_io_exit(inout: &mut VmInout, stats: &mut ExitStats, bus: &Bus) {
    stats.io_exits += 1;

    let port = u64::from(inout.port);
    let size = inout.bytes as usize;
    let mut data = inout.eax.to_le_bytes();
    let slice = &mut data[..size];

    if (inout.flags & INOUT_IN) != 0 {
        bus.read(port, slice);
        inout.eax = u32::from_le_bytes(data);
    } else {
        bus.write(port, slice);
    }
}

fn handle_mmio_exit(mmio: &mut api::VmMmio, stats: &mut ExitStats, bus: &Bus) {
    stats.mmio_exits += 1;

    let addr = mmio.gpa;
    let len = mmio.bytes as usize;
    let mut data = mmio.data.to_le_bytes();
    let slice = &mut data[..len];

    if mmio.read != 0 {
        bus.read(addr, slice);
        mmio.data = u64::from_le_bytes(data);
    } else {
        bus.write(addr, slice);
    }
}
