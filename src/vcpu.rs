use crate::bus::Bus;
use crate::config::Config;
use crate::input::Shutdown;
use crate::memory;
use crate::stats::ExitStats;
use crate::traits::Segment;
use crate::traits::Vcpu;

use ferrvm::printcrln;

/// Run one vCPU until it exits, then request shutdown for the whole VM.
pub fn run_vcpu(
    vcpu: &mut dyn Vcpu,
    cpu_id: u32,
    memory: &memory::GuestMemory,
    pio_bus: &Bus,
    mmio_bus: &Bus,
    shutdown: &Shutdown,
    config: &Config,
) -> Result<ExitStats, String> {
    let mut stats = ExitStats::new();
    let start_time = std::time::Instant::now();

    let result = loop {
        if shutdown.requested() {
            break Ok(());
        }

        // Run the vCPU until the next vm exit.
        let should_continue = match vcpu.step_run(&mut stats, memory, pio_bus, mmio_bus, config) {
            Ok(r) => r,
            // EINTR is a shutdown kick; EAGAIN means an AP is still waiting for SIPI.
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                ) =>
            {
                continue;
            }
            Err(e) => break Err(format!("vCPU {cpu_id} execution failed: {e}")),
        };

        if !should_continue {
            break Ok(());
        }
    };

    shutdown.request();

    let elapsed = start_time.elapsed();
    printcrln!("[vcpu {cpu_id}] execution stopped after {elapsed:?}");

    result.map(|()| stats)
}

fn dump_segment(name: &str, seg: &Segment) {
    printcrln!(
        "{}: sel=0x{:04X} base=0x{:016X} limit=0x{:08X} type={} p={} dpl={} db={} s={} l={} g={} unusable={}",
        name,
        seg.selector,
        seg.base,
        seg.limit,
        seg.type_,
        seg.present,
        seg.dpl,
        seg.db,
        seg.s,
        seg.l,
        seg.g,
        seg.unusable,
    );
}

pub fn dump_state(vcpu: &dyn Vcpu) -> Result<(), String> {
    let regs = vcpu
        .get_regs()
        .map_err(|e| format!("Failed to get regs: {e}"))?;
    let sregs = vcpu
        .get_sregs()
        .map_err(|e| format!("Failed to get sregs: {e}"))?;

    printcrln!("\n[debug] ========== vCPU State Dump ==========");

    printcrln!("[debug] General Purpose Registers:");
    printcrln!("[debug]   RAX=0x{:016X}  RBX=0x{:016X}", regs.rax, regs.rbx);
    printcrln!("[debug]   RCX=0x{:016X}  RDX=0x{:016X}", regs.rcx, regs.rdx);
    printcrln!("[debug]   RSI=0x{:016X}  RDI=0x{:016X}", regs.rsi, regs.rdi);
    printcrln!("[debug]   RSP=0x{:016X}  RBP=0x{:016X}", regs.rsp, regs.rbp);
    printcrln!("[debug]   R8 =0x{:016X}  R9 =0x{:016X}", regs.r8, regs.r9);
    printcrln!("[debug]   R10=0x{:016X}  R11=0x{:016X}", regs.r10, regs.r11);
    printcrln!("[debug]   R12=0x{:016X}  R13=0x{:016X}", regs.r12, regs.r13);
    printcrln!("[debug]   R14=0x{:016X}  R15=0x{:016X}", regs.r14, regs.r15);
    printcrln!("[debug]   RIP=0x{:016X}", regs.rip);
    printcrln!("[debug]   RFLAGS=0x{:016X}", regs.rflags);

    printcrln!("[debug] Control Registers:");
    printcrln!("[debug]   CR0=0x{:016X}", sregs.cr0);
    printcrln!("[debug]   CR2=0x{:016X}", sregs.cr2);
    printcrln!("[debug]   CR3=0x{:016X}", sregs.cr3);
    printcrln!("[debug]   CR4=0x{:016X}", sregs.cr4);
    printcrln!("[debug]   CR8=0x{:016X}", sregs.cr8);
    printcrln!("[debug]   EFER=0x{:016X}", sregs.efer);

    printcrln!("[debug] Segment Registers:");
    dump_segment("[debug]   CS ", &sregs.cs);
    dump_segment("[debug]   DS ", &sregs.ds);
    dump_segment("[debug]   ES ", &sregs.es);
    dump_segment("[debug]   FS ", &sregs.fs);
    dump_segment("[debug]   GS ", &sregs.gs);
    dump_segment("[debug]   SS ", &sregs.ss);
    dump_segment("[debug]   TR ", &sregs.tr);
    dump_segment("[debug]   LDT", &sregs.ldt);

    printcrln!("[debug] Descriptor Tables:");
    printcrln!(
        "[debug]   GDT: base=0x{:016X} limit=0x{:04X}",
        sregs.gdt.base,
        sregs.gdt.limit
    );
    printcrln!(
        "[debug]   IDT: base=0x{:016X} limit=0x{:04X}",
        sregs.idt.base,
        sregs.idt.limit
    );

    printcrln!("[debug] APIC Base: 0x{:016X}", sregs.apic_base);

    printcrln!("[debug] ========================================\n");

    Ok(())
}
