use super::api::{KVM_EXIT_IO_IN, KVM_EXIT_IO_OUT, KvmRun, VmExitReason};
use super::core::KvmVcpu;
use crate::bus::Bus;
use crate::config::Config;
use crate::stats::ExitStats;
use crate::vcpu::dump_state;
use ferrvm::printcrln;

pub fn handle_exit(
    kvm_run: &KvmRun,
    stats: &mut ExitStats,
    vcpu: &KvmVcpu,
    pio_bus: &Bus,
    mmio_bus: &Bus,
    _config: &Config,
) -> Result<bool, String> {
    let exit_reason = match kvm_run.exit_reason_enum() {
        Ok(reason) => reason,
        Err(code) => {
            printcrln!("[exit] Unknown exit code: {code}");
            stats.other_exits += 1;
            return Ok(true);
        }
    };

    match exit_reason {
        VmExitReason::Io => {
            handle_io_exit(kvm_run, stats, vcpu, pio_bus)?;
            Ok(true)
        }
        VmExitReason::MmIo => {
            handle_mmio_exit(kvm_run, stats, vcpu, mmio_bus);
            Ok(true)
        }
        VmExitReason::Hlt => {
            handle_hlt_exit(stats);
            Ok(true)
        }
        VmExitReason::Shutdown => {
            printcrln!("[exit] Guest shutdown detected");
            stats.shutdown_exits += 1;
            Ok(false)
        }
        VmExitReason::FailEntry => {
            handle_fail_entry(kvm_run, vcpu);
            stats.other_exits += 1;
            Ok(false)
        }
        VmExitReason::InternalError => {
            printcrln!("[exit] KVM internal error");
            stats.other_exits += 1;
            Ok(false)
        }
        _ => {
            printcrln!(
                "[exit] Unhandled exit: {} (code: {})",
                exit_reason,
                kvm_run.exit_reason
            );
            stats.other_exits += 1;
            Ok(true)
        }
    }
}

fn handle_fail_entry(kvm_run: &KvmRun, vcpu: &KvmVcpu) {
    // Parse fail_entry data from kvm_run
    // Layout: u64 hardware_entry_failure_reason, u32 cpu
    // SAFETY: exit reason is FailEntry, so the fail_entry union variant is active.
    let hardware_error = unsafe { kvm_run.exit.fail_entry.hardware_entry_failure_reason };
    // SAFETY: exit reason is FailEntry, so the fail_entry union variant is active.
    let cpu = unsafe { kvm_run.exit.fail_entry.cpu };

    // VMX instruction error codes (Intel SDM Vol 3C, Table 30-1)
    // The hardware_entry_failure_reason contains the VM-instruction error
    // in bits 0-31 (for VMX), or AMD-specific codes for SVM
    let vmx_error = (hardware_error & 0xFFFF_FFFF) as u32;
    let vmx_error_str = vmx_instruction_error_str(vmx_error);

    printcrln!("[fail_entry] ============================================");
    printcrln!("[fail_entry] vCPU failed to enter guest mode!");
    printcrln!("[fail_entry] CPU: {cpu}");
    printcrln!("[fail_entry] Hardware error code: 0x{hardware_error:016X}");
    printcrln!("[fail_entry] VMX instruction error: {vmx_error}");
    printcrln!("[fail_entry] Error: {vmx_error_str}");
    printcrln!("[fail_entry] ============================================");

    // Dump all CPU registers for diagnosis
    dump_state(vcpu).unwrap_or_else(|e| printcrln!("[fail_entry] Failed to dump vCPU state: {e}"));
}

/// VMX instruction error code to string
/// Intel SDM Vol 3C, Table 30-1
const fn vmx_instruction_error_str(error: u32) -> &'static str {
    match error {
        0 => "No error (unexpected)",
        1 => "VMCALL executed in VMX root operation",
        2 => "VMCLEAR with invalid physical address",
        3 => "VMCLEAR with VMXON pointer",
        4 => "VMLAUNCH with non-clear VMCS",
        5 => "VMRESUME with non-launched VMCS",
        6 => "VMRESUME after VMXOFF",
        7 => "VM entry with invalid control field(s)",
        8 => "VM entry with invalid host-state field(s)",
        9 => "VMPTRLD with invalid physical address",
        10 => "VMPTRLD with VMXON pointer",
        11 => "VMPTRLD with incorrect VMCS revision identifier",
        12 => "VMREAD/VMWRITE from/to unsupported VMCS component",
        13 => "VMWRITE to read-only VMCS component",
        15 => "VMXON executed in VMX root operation",
        16 => "VM entry with invalid executive-VMCS pointer",
        17 => "VM entry with non-launched executive VMCS",
        18 => "VM entry with executive-VMCS pointer not VMXON pointer",
        19 => "VMCALL with non-clear VMCS",
        20 => "VMCALL with invalid VM-exit control fields",
        22 => "VMCALL with incorrect MSEG revision identifier",
        23 => "VMXOFF under dual-monitor treatment of SMIs and SMM",
        24 => "VMCALL with invalid SMM-monitor features",
        25 => "VM entry with invalid VM-execution control fields in executive VMCS",
        26 => "VM entry with events blocked by MOV SS",
        28 => "Invalid operand to INVEPT/INVVPID",
        _ => "Unknown VMX error",
    }
}

/// Handle I/O exit (IN/OUT instructions)
fn handle_io_exit(
    kvm_run: &KvmRun,
    stats: &mut ExitStats,
    vcpu: &KvmVcpu,
    bus: &Bus,
) -> Result<(), String> {
    let io_exit = kvm_run
        .io_exit()
        .ok_or_else(|| "Failed to parse IO exit".to_string())?;

    stats.io_exits += 1;

    let run_start = vcpu.kvm_run.as_ptr();
    let io = io_exit;

    let port = u64::from(io.port);
    let size = io.size as usize; // 1, 2, or 4
    let count = io.count as usize; // usually 1; >1 for INS/OUTS
    let base = run_start.cast::<u8>();
    // SAFETY: data_offset is within the kvm_run mmap page, set by the kernel.
    let data = unsafe { base.add(io.data_offset as usize) };

    for i in 0..count {
        // SAFETY: data..data+count*size lies within the kvm_run page; each chunk is size bytes.
        let slice = unsafe { std::slice::from_raw_parts_mut(data.add(i * size), size) };
        match io.direction {
            KVM_EXIT_IO_IN => {
                bus.read(port, slice);
            }
            KVM_EXIT_IO_OUT => {
                bus.write(port, slice);
            }
            _ => {}
        }
    }

    Ok(())
}

/// Handle MMIO exit (memory-mapped I/O)
fn handle_mmio_exit(_kvm_run: &KvmRun, stats: &mut ExitStats, vcpu: &KvmVcpu, bus: &Bus) {
    stats.mmio_exits += 1;

    // Mutate the kvm_run.exit.mmio data buffer in-place to ensure read results
    // are correctly written back into the guest shared page.
    // SAFETY: kvm_run points to the live mmap page; exit reason is MmIo so the mmio variant is active.
    let mmio = unsafe { &mut (*vcpu.kvm_run.as_ptr()).exit.mmio };
    let addr = mmio.phys_addr;
    let len = (mmio.len as usize).min(8);
    let is_write = mmio.is_write;

    if is_write != 0 {
        let data = &mmio.data[..len];
        bus.write(addr, data);
    } else {
        let data = &mut mmio.data[..len];
        bus.read(addr, data);
    }
}

/// Handle HLT exit (guest executed HLT instruction)
const fn handle_hlt_exit(stats: &mut ExitStats) {
    stats.hlt_exits += 1;

    // The guest executed HLT - it's waiting for an interrupt
    // We could implement proper interrupt handling here, but for now
    // we just continue (the kernel will handle pending interrupts)
}
