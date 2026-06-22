//! Minimal Intel MP table for guests without ACPI or firmware.
//! It describes the CPUs, I/O APIC, and legacy IRQ routing.

use crate::memory::GuestMemory;

/// MP floating pointer address in the BIOS scan window, reserved in e820.
const MPF_ADDR: u64 = 0x000F_0000;
/// The configuration table follows immediately after the 16-byte pointer.
const MPC_ADDR: u64 = MPF_ADDR + 16;

/// Local APIC MMIO base, as KVM's in-kernel LAPIC exposes it.
const APIC_DEFAULT_PHYS_BASE: u32 = 0xFEE0_0000;
/// I/O APIC MMIO base, as KVM's in-kernel IOAPIC exposes it.
const IOAPIC_DEFAULT_PHYS_BASE: u32 = 0xFEC0_0000;

// MP configuration table entry types.
const MP_PROCESSOR: u8 = 0;
const MP_BUS: u8 = 1;
const MP_IOAPIC: u8 = 2;
const MP_INTSRC: u8 = 3;
const MP_LINTSRC: u8 = 4;

// Interrupt types used in the INTSRC / LINTSRC entries.
const MP_IRQ_INT: u8 = 0;
const MP_IRQ_NMI: u8 = 1;
const MP_IRQ_EXTINT: u8 = 3;

// Processor entry CPU flags.
const CPU_ENABLED: u8 = 1;
const CPU_BOOTPROCESSOR: u8 = 2;

/// I/O APIC entry flag: this APIC is usable.
const MPC_APIC_USABLE: u8 = 1;

/// Local APIC version reported in processor entries (integrated APIC).
const LAPIC_VERSION: u8 = 0x14;
/// I/O APIC version reported in the I/O APIC entry.
const IOAPIC_VERSION: u8 = 0x11;

/// Number of legacy ISA interrupt lines we describe.
const NUM_ISA_IRQS: u8 = 16;

/// KVM routes the PIT to pin 0; bhyve uses pin 2, with ExtINT on pin 0.
#[cfg(target_os = "illumos")]
const TIMER_ON_PIN2: bool = true;
#[cfg(not(target_os = "illumos"))]
const TIMER_ON_PIN2: bool = false;

/// Size of the MP configuration table header, in bytes.
const MPC_HEADER_LEN: usize = 44;

fn push_u16(buf: &mut Vec<u8>, value: u16) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(buf: &mut Vec<u8>, value: u32) {
    buf.extend_from_slice(&value.to_le_bytes());
}

/// Return the checksum byte that makes the structure's byte sum zero.
fn checksum(bytes: &[u8]) -> u8 {
    let sum = bytes.iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
    0u8.wrapping_sub(sum)
}

/// Read the host CPU signature and features for the MP processor entries.
fn cpu_signature_and_features() -> (u32, u32) {
    // CPUID leaf 1 is always available on x86_64, so this is a safe intrinsic.
    let leaf = std::arch::x86_64::__cpuid(1);
    (leaf.eax, leaf.edx)
}

/// Write MP tables for `num_cpus`; CPU 0 is the bootstrap processor.
pub fn write_mptable(mem: &GuestMemory, num_cpus: u8) -> Result<(), String> {
    let (cpu_sig, cpu_feat) = cpu_signature_and_features();
    // Keep the I/O APIC id distinct from the CPU APIC ids.
    let ioapic_id = num_cpus;

    let mut mpc = Vec::new();

    // --- Configuration table header (length/count/checksum backfilled) ---
    mpc.extend_from_slice(b"PCMP"); // signature
    push_u16(&mut mpc, 0); // [4..6]  base table length (placeholder)
    mpc.push(4); //              [6]    spec revision 1.4
    mpc.push(0); //              [7]    checksum (placeholder)
    mpc.extend_from_slice(b"FERRVM00"); // [8..16]  OEM id
    mpc.extend_from_slice(b"ferrvm 0.1  "); // [16..28] product id
    push_u32(&mut mpc, 0); //    [28..32] OEM table pointer
    push_u16(&mut mpc, 0); //    [32..34] OEM table size
    push_u16(&mut mpc, 0); //    [34..36] entry count (placeholder)
    push_u32(&mut mpc, APIC_DEFAULT_PHYS_BASE); // [36..40] local APIC address
    push_u32(&mut mpc, 0); //    [40..44] reserved
    debug_assert_eq!(mpc.len(), MPC_HEADER_LEN);

    let mut entries: u16 = 0;

    // Processor entries, one per CPU.
    for id in 0..num_cpus {
        let mut flags = CPU_ENABLED;
        if id == 0 {
            flags |= CPU_BOOTPROCESSOR;
        }
        mpc.push(MP_PROCESSOR);
        mpc.push(id); // local APIC id
        mpc.push(LAPIC_VERSION);
        mpc.push(flags);
        push_u32(&mut mpc, cpu_sig);
        push_u32(&mut mpc, cpu_feat);
        push_u32(&mut mpc, 0); // reserved
        push_u32(&mut mpc, 0); // reserved
        entries += 1;
    }

    // A single ISA bus.
    mpc.push(MP_BUS);
    mpc.push(0); // bus id 0
    mpc.extend_from_slice(b"ISA   "); // bus type string (6 bytes)
    entries += 1;

    // The I/O APIC.
    mpc.push(MP_IOAPIC);
    mpc.push(ioapic_id);
    mpc.push(IOAPIC_VERSION);
    mpc.push(MPC_APIC_USABLE);
    push_u32(&mut mpc, IOAPIC_DEFAULT_PHYS_BASE);
    entries += 1;

    // Route ISA IRQs to matching pins, except for bhyve's PIT/ExtINT wiring.
    for pin in 0..NUM_ISA_IRQS {
        let (int_type, src_irq) = if TIMER_ON_PIN2 {
            match pin {
                0 => (MP_IRQ_EXTINT, 0), // 8259 ExtINT on pin 0
                2 => (MP_IRQ_INT, 0),    // 8254 timer (ISA IRQ 0) on pin 2
                _ => (MP_IRQ_INT, pin),
            }
        } else {
            (MP_IRQ_INT, pin) // identity mapping (KVM)
        };
        mpc.push(MP_INTSRC);
        mpc.push(int_type);
        push_u16(&mut mpc, 0); // conforming polarity + trigger
        mpc.push(0); // source bus id (the ISA bus above)
        mpc.push(src_irq); // source bus IRQ
        mpc.push(ioapic_id); // destination I/O APIC id
        mpc.push(pin); // destination I/O APIC pin
        entries += 1;
    }

    // Route 8259 ExtINT to LINT0 and NMI to LINT1 on every CPU.
    mpc.push(MP_LINTSRC);
    mpc.push(MP_IRQ_EXTINT);
    push_u16(&mut mpc, 0);
    mpc.push(0); // source bus id
    mpc.push(0); // source bus IRQ
    mpc.push(0xff); // destination APIC id: all
    mpc.push(0); // LINT0
    entries += 1;

    mpc.push(MP_LINTSRC);
    mpc.push(MP_IRQ_NMI);
    push_u16(&mut mpc, 0);
    mpc.push(0);
    mpc.push(0);
    mpc.push(0xff); // destination APIC id: all
    mpc.push(1); // LINT1
    entries += 1;

    // Backfill length, entry count, and checksum now that the table is built.
    let length =
        u16::try_from(mpc.len()).map_err(|_| "MP configuration table too large".to_string())?;
    mpc[4..6].copy_from_slice(&length.to_le_bytes());
    mpc[34..36].copy_from_slice(&entries.to_le_bytes());
    mpc[7] = checksum(&mpc);

    // --- Floating pointer structure (16 bytes) ---
    let mut mpf = [0u8; 16];
    mpf[0..4].copy_from_slice(b"_MP_");
    mpf[4..8].copy_from_slice(&(MPC_ADDR as u32).to_le_bytes()); // config table address
    mpf[8] = 1; // length in 16-byte paragraphs
    mpf[9] = 4; // spec revision 1.4
    // mpf[11] (feature1) = 0: configuration table present, virtual-wire mode.
    mpf[10] = checksum(&mpf);

    mem.write_at(MPC_ADDR, &mpc)
        .map_err(|e| format!("Failed to write MP configuration table: {e}"))?;
    mem.write_at(MPF_ADDR, &mpf)
        .map_err(|e| format!("Failed to write MP floating pointer: {e}"))?;

    Ok(())
}
