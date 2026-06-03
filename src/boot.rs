use std::fs::File;
use std::io::{Error, ErrorKind, Read, Seek, SeekFrom};
use std::path::Path;

use crate::bootparams::{
    BootParams, E820_RAM, E820_RESERVED, E820Entry, KERNEL_BOOT_FLAG_MAGIC_NUMBER,
    KERNEL_HDR_MAGIC_NUMBER, KERNEL_MIN_ALIGNMENT_BYTES, KERNEL_TYPE_OF_LOADER, SetupHeader,
};
use crate::elf::{Elf64Header, Elf64Phdr, ElfClass};
use crate::memory::GuestMemory;
use crate::traits::Segment;
use crate::traits::Vcpu;

use ferrvm::printcrln;

const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];

// Address where we put the kernel command line
pub const CMDLINE_ADDR: u64 = 0x0002_0000;

// EFER flags (Extended Feature Enable Register)
pub const EFER_LME: u64 = 1 << 8; // Long Mode Enable
pub const EFER_LMA: u64 = 1 << 10; // Long Mode Active
pub const EFER_NXE: u64 = 1 << 11; // No Execute Enable

const BOOT_STACK_POINTER: u64 = 0x8ff0;
const ZERO_PAGE_START: u64 = 0x7000;
const EBDA_START: u64 = 0x9fc00;
const HIMEM_START: u64 = 0x0010_0000; // kernel load address in 64-bit mode (16MB)
const NORMAL_VGA_MODE: u16 = 0xFFFF;
const LINUX_BOOT_HDR_LOAD_HIGH: u8 = 1 << 0;
const LINUX_BOOT_HDR_KEEP_SEGMENTS: u8 = 1 << 6;
const LINUX_BOOT_HDR_CAN_USE_HEAP: u8 = 1 << 7;
const LINUX_HEAP_END_PTR: u16 = 0xFE00;
const SETUP_HEADER_OFFSET: usize = 0x1F1;
const INITRAMFS_ADDR: u64 = 0x800_0000; // 128 MB

fn is_elf64(data: &[u8]) -> bool {
    data.len() >= 5
        && data[0] == 0x7F
        && data[1] == b'E'
        && data[2] == b'L'
        && data[3] == b'F'
        && data[4] == 2
}

fn is_bzimage(data: &[u8]) -> bool {
    data[0x1FE] == 0x55 && data[0x1FF] == 0xAA && &data[0x202..0x206] == b"HdrS" // setup header magic
        && u16::from_le_bytes([data[0x206], data[0x207]]) >= 0x0200 // protocol >= 2.00
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KernelImageFormat {
    BzImage,
    Elf64,
}

fn detect_kernel_image_format(data: &[u8]) -> Result<KernelImageFormat, String> {
    if is_elf64(data) {
        return Ok(KernelImageFormat::Elf64);
    }

    if is_bzimage(data) {
        return Ok(KernelImageFormat::BzImage);
    }

    Err("Unsupported kernel image format: expected ELF64 or Linux bzImage boot image".to_string())
}

fn get_setup_sects(kernel_data: &[u8]) -> u8 {
    // setup_sects is at file offset 0x1F1, and specifies the size of the setup header
    // in 512-byte sectors (not counting the first sector) if 0, assume 4 (default)
    let setup_sects = kernel_data[0x1F1];
    printcrln!("[boot] setup_sects (at 0x1F1): {setup_sects}");

    if setup_sects == 0 {
        printcrln!("[boot] setup_sects is 0, using default of 4");
        4
    } else {
        setup_sects
    }
}

fn load_initrd(mem: &GuestMemory, initrd_path: Option<&str>) -> Result<(u32, u32), String> {
    let Some(resolved_path) = initrd_path else {
        printcrln!("[boot] --initramfs not provided, skipping initrd loading");
        return Ok((0, 0));
    };

    if !Path::new(resolved_path).exists() {
        printcrln!("[boot] No initrd found at {resolved_path}, skipping");
        return Ok((0, 0));
    }

    let initrd_content = std::fs::read(resolved_path)
        .map_err(|e| format!("Failed to read initrd file: {resolved_path}: {e}"))?;
    let initrd_size = u32::try_from(initrd_content.len())
        .map_err(|_| "Initrd is too large for 32-bit boot protocol fields".to_string())?;

    let initrd_addr = INITRAMFS_ADDR;

    printcrln!(
        "[boot] Loading initrd from {} ({} bytes) at 0x{:X}",
        resolved_path,
        initrd_content.len(),
        initrd_addr
    );

    mem.write_at(initrd_addr, &initrd_content)
        .map_err(|e| format!("Failed to write initrd to guest memory: {e}"))?;

    Ok((initrd_addr as u32, initrd_size))
}

pub fn load_kernel(
    mem: &GuestMemory,
    vcpu: &dyn Vcpu,
    kernel_path: &str,
    cmdline: &str,
    initrd_path: Option<&str>,
) -> Result<(), String> {
    printcrln!("[boot] Loading kernel from: {kernel_path}");

    let mut kernel_file = File::open(kernel_path)
        .map_err(|e| format!("Failed to open kernel: {kernel_path}: {e}"))?;

    let mut kernel_data = Vec::new();
    kernel_file
        .read_to_end(&mut kernel_data)
        .map_err(|e| format!("Failed to read kernel file: {e}"))?;

    printcrln!(
        "[boot] Kernel file size: {} bytes ({} MB)",
        kernel_data.len(),
        kernel_data.len() / (1024 * 1024)
    );

    let kernel_format = detect_kernel_image_format(&kernel_data)
        .map_err(|e| format!("Failed to detect kernel image format: {e}"))?;

    let kernel_entry = match kernel_format {
        KernelImageFormat::BzImage => {
            let setup_sects = get_setup_sects(&kernel_data);

            printcrln!("[boot] Detected bzImage kernel");
            printcrln!("[boot] Setup sectors: {setup_sects}");

            let payload_offset = (setup_sects as usize + 1) * 512;

            if payload_offset >= kernel_data.len() {
                Err(format!(
                    "Invalid kernel: payload offset {} exceeds kernel size {}",
                    payload_offset,
                    kernel_data.len()
                ))?;
            }

            printcrln!(
                "[boot] Loading kernel payload ({} bytes) at 0x{HIMEM_START:X}",
                kernel_data.len() - payload_offset
            );

            mem.write_at(HIMEM_START, &kernel_data[payload_offset..])
                .map_err(|e| format!("Failed to write kernel to guest memory: {e}"))?;

            HIMEM_START
        }
        KernelImageFormat::Elf64 => load_elf_kernel(mem, &mut kernel_file, HIMEM_START)
            .map_err(|e| format!("Failed to load ELF kernel with second loader: {e}"))?,
    };

    let (initrd_addr, initrd_size) = load_initrd(mem, initrd_path)?;

    setup_boot_params(
        mem,
        kernel_format,
        &kernel_data,
        cmdline,
        initrd_addr,
        initrd_size,
    )
    .map_err(|e| format!("Failed to set up boot parameters: {e}"))?;

    setup_cmdline(mem, cmdline).map_err(|e| format!("Failed to set up command line: {e}"))?;

    setup_general_registers(vcpu, kernel_entry)
        .map_err(|e| format!("Failed to set up general registers: {e}"))?;

    match kernel_format {
        KernelImageFormat::BzImage => setup_protected_mode(vcpu)
            .map_err(|e| format!("Failed to set up protected mode for bzImage: {e}"))?,
        KernelImageFormat::Elf64 => {
            setup_long_mode(vcpu, mem).map_err(|e| format!("Failed to set up long mode: {e}"))?;
        }
    }

    printcrln!("[boot] Kernel loaded successfully!");
    Ok(())
}

fn read_struct<T: Copy>(file: &mut File) -> std::io::Result<T> {
    let size = core::mem::size_of::<T>();
    let mut buf = vec![0u8; size];
    file.read_exact(&mut buf)?;
    // SAFETY: buf is sized to size_of::<T>() and fully initialized by read_exact; read is unaligned.
    Ok(unsafe { core::ptr::read_unaligned(buf.as_ptr().cast::<T>()) })
}

/// A minimal standalone ELF loader for `x86_64`.
/// Returns the kernel entry point on success.
fn load_elf_kernel(
    guest_mem: &GuestMemory,
    kernel_file: &mut File,
    himem_start: u64,
) -> std::io::Result<u64> {
    // 1. Read the ELF64 header
    kernel_file.seek(SeekFrom::Start(0))?;
    let ehdr: Elf64Header = read_struct(kernel_file)?;

    if ehdr.e_ident_magic != ELF_MAGIC {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "Invalid ELF magic number",
        ));
    }

    if ehdr.e_ident_class != ElfClass::Elf64 as u8 {
        return Err(Error::new(ErrorKind::InvalidData, "Not a 64-bit ELF file"));
    }

    // 2. Iterate over program headers
    for i in 0..ehdr.e_phnum {
        let phdr_offset = ehdr.e_phoff + (u64::from(i) * u64::from(ehdr.e_phentsize));
        kernel_file.seek(SeekFrom::Start(phdr_offset))?;
        let phdr: Elf64Phdr = read_struct(kernel_file)?;

        if !phdr.is_load() {
            continue;
        }

        let load_addr = (phdr.p_paddr as usize).max(himem_start as usize);
        let end_addr = load_addr + phdr.p_memsz as usize;

        if end_addr > guest_mem.size() as usize {
            return Err(Error::new(
                ErrorKind::OutOfMemory,
                "Guest memory too small for ELF segment",
            ));
        }

        // Copy file data to guest memory
        if phdr.p_filesz > 0 {
            kernel_file.seek(SeekFrom::Start(phdr.p_offset))?;
            let mut segment = vec![0u8; phdr.p_filesz as usize];
            kernel_file.read_exact(&mut segment)?;
            guest_mem
                .write_at(load_addr as u64, &segment)
                .map_err(Error::other)?;
        }

        // Zero out .bss
        if phdr.p_memsz > phdr.p_filesz {
            let bss_start = load_addr + phdr.p_filesz as usize;
            let bss_len = (phdr.p_memsz - phdr.p_filesz) as usize;
            let zeros = vec![0u8; bss_len];
            guest_mem
                .write_at(bss_start as u64, &zeros)
                .map_err(Error::other)?;
        }
    }

    Ok(ehdr.e_entry)
}

/// Set up the kernel command line in guest memory
fn setup_cmdline(mem: &GuestMemory, cmdline: &str) -> Result<(), String> {
    // Command line must be null-terminated
    let mut cmdline_bytes = cmdline.as_bytes().to_vec();
    cmdline_bytes.push(0);

    if cmdline_bytes.len() > 2048 {
        return Err(format!(
            "Command line too long: {} bytes (max 2048)",
            cmdline_bytes.len()
        ));
    }

    mem.write_at(CMDLINE_ADDR, &cmdline_bytes)
        .map_err(|e| format!("Failed to write command line: {e}"))?;

    printcrln!(
        "[boot] Command line at 0x{:X}: {} bytes",
        CMDLINE_ADDR,
        cmdline_bytes.len()
    );

    Ok(())
}

fn read_setup_header(kernel_data: &[u8]) -> Result<SetupHeader, String> {
    let start = SETUP_HEADER_OFFSET;
    let end = start + size_of::<SetupHeader>();
    let header = kernel_data
        .get(start..end)
        .ok_or_else(|| "Kernel image too small for setup header".to_string())?;

    // SAFETY: header spans exactly size_of::<SetupHeader>() bytes, checked by get above; read is unaligned.
    Ok(unsafe { std::ptr::read_unaligned(header.as_ptr().cast::<SetupHeader>()) })
}

fn setup_boot_params(
    mem: &GuestMemory,
    image_format: KernelImageFormat,
    kernel_data: &[u8],
    cmdline: &str,
    initrd_addr: u32,
    initrd_size: u32,
) -> Result<(), String> {
    let mut boot_params = BootParams::new();

    boot_params.hdr.type_of_loader = KERNEL_TYPE_OF_LOADER;
    boot_params.hdr.boot_flag = KERNEL_BOOT_FLAG_MAGIC_NUMBER;
    boot_params.hdr.header = KERNEL_HDR_MAGIC_NUMBER;
    boot_params.hdr.cmd_line_ptr = CMDLINE_ADDR as u32;
    boot_params.hdr.cmdline_size = 1 + cmdline.len() as u32;
    boot_params.hdr.kernel_alignment = KERNEL_MIN_ALIGNMENT_BYTES;

    // 0x0000_0000  ┌─────────────────────┐
    //              │  Low RAM (E820_RAM) │  ← entry 1: 0 → EBDA_START
    // 0x0009_FC00  ├─────────────────────┤
    //              │  EBDA / reserved    │  ← entry 2: EBDA_START → HIMEM_START
    // 0x0010_0000  ├─────────────────────┤
    //              │  High RAM below gap │  ← entry 3: HIMEM_START → MMIO_MEM_START
    // 0xD000_0000  ├─────────────────────┤
    //              │  MMIO hole (32-bit) │  ← NO entry
    // 0xFFFF_FFFF  │                     │
    // 1_0000_0000  ├─────────────────────┤
    //              │  RAM above 4 GiB    │  ← entry 4: FIRST_ADDR_PAST_32BITS → last_addr (not implemented yet)
    //              └─────────────────────┘

    let last_addr = mem.size();
    let e820_table: Vec<E820Entry> = vec![
        // Usable low memory: 0 to 0x9FC00 (below EBDA)
        E820Entry {
            addr: 0,
            size: EBDA_START,
            entry_type: E820_RAM,
        },
        // Reserved: 0x9FC00 to 0xFFFFF (EBDA + ROM + video)
        E820Entry {
            addr: EBDA_START,
            size: HIMEM_START - EBDA_START,
            entry_type: E820_RESERVED,
        },
        // Usable: 1MB to end of RAM (assuming RAM is less than 4GB for now)
        E820Entry {
            addr: HIMEM_START,
            size: last_addr - HIMEM_START,
            entry_type: E820_RAM,
        },
    ];

    for (i, entry) in e820_table.iter().enumerate() {
        boot_params.e820_table[i] = *entry;
    }
    boot_params.e820_entries = e820_table.len() as u8;

    if image_format == KernelImageFormat::BzImage {
        boot_params.hdr = read_setup_header(kernel_data)
            .map_err(|e| format!("Failed to read bzImage setup header for boot_params: {e}"))?;
        boot_params.hdr.vid_mode = NORMAL_VGA_MODE;
        boot_params.hdr.type_of_loader = 0xFF;
        boot_params.hdr.loadflags |=
            LINUX_BOOT_HDR_LOAD_HIGH | LINUX_BOOT_HDR_KEEP_SEGMENTS | LINUX_BOOT_HDR_CAN_USE_HEAP;
        boot_params.hdr.heap_end_ptr = LINUX_HEAP_END_PTR;
        boot_params.hdr.cmd_line_ptr = CMDLINE_ADDR as u32;
        boot_params.hdr.cmdline_size = (cmdline.len() + 1) as u32;
        boot_params.hdr.code32_start = HIMEM_START as u32;
    }

    boot_params.hdr.ramdisk_image = initrd_addr;
    boot_params.hdr.ramdisk_size = initrd_size;

    // SAFETY: boot_params is a fully initialized BootParams, so write_obj may read its bytes.
    unsafe {
        mem.write_obj(ZERO_PAGE_START, &boot_params)
            .map_err(|e| format!("Failed to write boot_params to guest memory: {e}"))?;
    }

    let type_of_loader = boot_params.hdr.type_of_loader;
    let loadflags = boot_params.hdr.loadflags;
    let cmd_line_ptr = boot_params.hdr.cmd_line_ptr;
    let cmdline_size = boot_params.hdr.cmdline_size;

    printcrln!("[boot] boot_params configured at 0x{ZERO_PAGE_START:X}");
    printcrln!("  type_of_loader = 0x{type_of_loader:X}");
    printcrln!("  loadflags      = 0x{loadflags:02X}");
    printcrln!("  cmd_line_ptr   = 0x{cmd_line_ptr:08X}");
    printcrln!("  cmdline_size   = {cmdline_size}");
    printcrln!("  e820_entries   = {}", boot_params.e820_entries);

    Ok(())
}

const fn generate_segment(selector_index: u16, type_: u8) -> Segment {
    Segment {
        base: 0,
        limit: 0x000f_ffff,
        selector: selector_index << 3,
        type_,
        present: 1,
        dpl: 0,
        db: 0,
        s: 1,
        l: 1,
        g: 1,
        avl: 0,
        unusable: 0,
        padding: 0,
    }
}
const fn segment_gdt_entry(seg: &Segment) -> u64 {
    let base = seg.base;
    let limit = seg.limit as u64;
    let flags = (seg.g as u64 & 0x1) << 3
        | (seg.db as u64 & 0x1) << 2
        | (seg.l as u64 & 0x1) << 1
        | (seg.avl as u64 & 0x1);
    let access = (seg.present as u64 & 0x1) << 7
        | (seg.dpl as u64 & 0b11) << 5
        | (seg.s as u64 & 0x1) << 4
        | (seg.type_ as u64 & 0b1111);
    ((base & 0xff00_0000u64) << 32)
        | ((base & 0x00ff_ffffu64) << 16)
        | (limit & 0x0000_ffffu64)
        | ((limit & 0x000f_0000u64) << 32)
        | (flags << 52)
        | (access << 40)
}

const BOOT_GDT_OFFSET: u64 = 0x500;
const X86_CR0_PE: u64 = 0x1;
const X86_CR4_PAE: u64 = 0x20;
const X86_CR0_PG: u64 = 0x8000_0000;

fn setup_long_mode(vcpu: &dyn Vcpu, mem: &GuestMemory) -> Result<(), String> {
    const CODE_SEG: Segment = generate_segment(1, 0b1011); // 0b1011: Code, Executed/Read, accessed
    const DATA_SEG: Segment = generate_segment(2, 0b0011); // 0b0011: Data, Read/Write, accessed

    // Get current CPU state
    let mut sregs = vcpu
        .get_sregs()
        .map_err(|e| format!("Failed to get CPU segment registers: {e}"))?;

    // construct segment and set to segment registers
    sregs.cs = CODE_SEG;
    sregs.ds = DATA_SEG;
    sregs.es = DATA_SEG;
    sregs.fs = DATA_SEG;
    sregs.gs = DATA_SEG;
    sregs.ss = DATA_SEG;

    // construct gdt table, write to memory and set it to register
    let gdt_table: [u64; 3] = [
        0,                            // NULL
        segment_gdt_entry(&CODE_SEG), // CODE
        segment_gdt_entry(&DATA_SEG), // DATA
    ];

    for (index, entry) in gdt_table.iter().enumerate() {
        // SAFETY: entry is a valid u64, so write_obj may read its bytes.
        unsafe {
            mem.write_obj(
                BOOT_GDT_OFFSET + (index * std::mem::size_of::<u64>()) as u64,
                entry,
            )
            .map_err(|e| format!("Failed to write GDT entry {index}: {e}"))?;
        }
    }
    sregs.gdt.base = BOOT_GDT_OFFSET;
    sregs.gdt.limit = std::mem::size_of_val(&gdt_table) as u16 - 1;

    // enable protected mode
    sregs.cr0 |= X86_CR0_PE;

    // page tables: identity map the first 1GB of memory using 2MB pages
    let boot_pml4_addr = 0xa000_u64;
    let boot_pdpte_addr = 0xb000_u64;
    let boot_pde_addr = 0xc000_u64;

    // PML4 table: one entry pointing to PDPT
    // Entry format: [address_bits | flags]
    // Flags: bit 0 = present, bit 1 = writable
    mem.write_at(boot_pml4_addr, &(boot_pdpte_addr | 0b11).to_le_bytes())
        .map_err(|e| format!("Failed to write PML4 entry: {e}"))?;

    // PDPT table: one entry pointing to PD
    mem.write_at(boot_pdpte_addr, &(boot_pde_addr | 0b11).to_le_bytes())
        .map_err(|e| format!("Failed to write PDPT entry: {e}"))?;

    // PD table: 512 entries, each mapping 2MB (huge page)
    // Huge page flag is bit 7 (PS = Page Size)
    for i in 0..512u64 {
        // Each entry maps 2MB: (i * 2MB) → (i * 2MB)
        // Flags: bit 0 = present, bit 1 = writable, bit 7 = huge page (2MB)
        let offset = boot_pde_addr + (i * 8);
        mem.write_at(offset, &((i << 21) | 0b1000_0011_u64).to_le_bytes())
            .map_err(|e| format!("Failed to write PD entry: {e}"))?;
    }

    printcrln!(
        "[boot] Page tables set up: PML4=0x{boot_pml4_addr:X}, PDPT=0x{boot_pdpte_addr:X}, PD=0x{boot_pde_addr:X}"
    );

    sregs.cr3 = boot_pml4_addr;
    sregs.cr4 |= X86_CR4_PAE;
    sregs.cr0 |= X86_CR0_PG;
    sregs.efer |= EFER_LMA | EFER_LME;

    // Update CPU state
    vcpu.set_sregs(&sregs)
        .map_err(|e| format!("Failed to set CPU segment registers: {e}"))?;

    Ok(())
}

const fn make_flat32_segment(seg: &mut Segment, type_: u8) {
    seg.base = 0;
    seg.limit = u32::MAX;
    seg.g = 1;
    seg.db = 1;
    seg.l = 0;
    seg.type_ = type_;
}

fn setup_protected_mode(vcpu: &dyn Vcpu) -> Result<(), String> {
    let mut sregs = vcpu
        .get_sregs()
        .map_err(|e| format!("Failed to get CPU segment registers: {e}"))?;

    make_flat32_segment(&mut sregs.cs, 0b1011); // Code segment: Executable, Readable, Accessed
    make_flat32_segment(&mut sregs.ds, 0b0011); // Data segment: Read/Write, Accessed
    make_flat32_segment(&mut sregs.es, 0b0011); // Data segment: Read/Write, Accessed
    make_flat32_segment(&mut sregs.fs, 0b0011); // Data segment: Read/Write, Accessed
    make_flat32_segment(&mut sregs.gs, 0b0011); // Data segment: Read/Write, Accessed
    make_flat32_segment(&mut sregs.ss, 0b0011); // Data segment: Read/Write, Accessed

    sregs.cr0 |= X86_CR0_PE;
    sregs.cr0 &= !X86_CR0_PG;
    sregs.cr4 &= !X86_CR4_PAE;
    sregs.efer &= !(EFER_LME | EFER_LMA | EFER_NXE);

    vcpu.set_sregs(&sregs)
        .map_err(|e| format!("Failed to set CPU segment registers for bzImage: {e}"))?;

    Ok(())
}

fn setup_general_registers(vcpu: &dyn Vcpu, entry_point: u64) -> Result<(), String> {
    let mut regs = vcpu
        .get_regs()
        .map_err(|e| format!("Failed to get CPU general registers: {e}"))?;

    regs.rip = entry_point;
    regs.rsp = BOOT_STACK_POINTER;
    regs.rbp = BOOT_STACK_POINTER;
    regs.rsi = ZERO_PAGE_START;

    // RFLAGS: bit 1 is reserved (always 1), but clear bit 9 (IF - interrupt flag)
    // Some VMX implementations are stricter about initial RFLAGS
    regs.rflags = 0x2; // Bit 1 set, interrupts disabled (IF=0)

    vcpu.set_regs(&regs)
        .map_err(|e| format!("Failed to set CPU general registers: {e}"))?;

    printcrln!(
        "[boot] General registers configured: RIP=0x{:X}, RSI=0x{:X}, RSP=0x{:X}, RFLAGS=0x{:X}",
        regs.rip,
        regs.rsi,
        regs.rsp,
        regs.rflags
    );

    Ok(())
}
