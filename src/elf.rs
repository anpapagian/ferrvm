/// ELF class (32-bit or 64-bit)
#[allow(dead_code)]
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfClass {
    None = 0,
    Elf32 = 1,
    Elf64 = 2,
}

/// Data encoding (endianness)
#[allow(dead_code)]
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfData {
    None = 0,
    Lsb = 1, // Little-endian
    Msb = 2, // Big-endian
}

/// OS/ABI identification
#[allow(dead_code)]
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsAbi {
    None = 0,
    HpUx = 1,
    NetBsd = 2,
    Linux = 3,
    Solaris = 6,
    Aix = 7,
    FreeBsd = 9,
    OpenBsd = 12,
}

/// Object file type
#[allow(dead_code)]
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfType {
    None = 0,
    Rel = 1,  // Relocatable
    Exec = 2, // Executable
    Dyn = 3,  // Shared object
    Core = 4, // Core dump
}

/// Machine architecture
#[allow(dead_code)]
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfMachine {
    None = 0,
    Sparc = 2,
    X86 = 3,
    Mips = 8,
    PowerPc = 20,
    Arm = 40,
    X86_64 = 62,
    AArch64 = 183,
    RiscV = 243,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf64Header {
    pub e_ident_magic: [u8; 4],
    pub e_ident_class: u8, // ElfClass
    pub e_ident_data: u8,  // ElfData
    pub e_ident_version: u8,
    pub e_ident_osabi: u8, // OsAbi
    pub e_ident_abiversion: u8,
    pub e_ident_pad: [u8; 7],
    pub e_type: u16,    // ElfType
    pub e_machine: u16, // ElfMachine
    pub e_version: u32,
    pub e_entry: u64,
    pub e_phoff: u64,
    pub e_shoff: u64,
    pub e_flags: u32,
    pub e_ehsize: u16,
    pub e_phentsize: u16,
    pub e_phnum: u16,
    pub e_shentsize: u16,
    pub e_shnum: u16,
    pub e_shstrndx: u16,
}

/// Program header segment type
#[allow(dead_code)]
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhType {
    Null = 0,    // Unused entry
    Load = 1,    // Loadable segment
    Dynamic = 2, // Dynamic linking info
    Interp = 3,  // Path to interpreter
    Note = 4,    // Auxiliary information
    Shlib = 5,   // Reserved
    Phdr = 6,    // Program header table
    Tls = 7,     // Thread-local storage
    GnuEhFrame = 0x6474_E550,
    GnuStack = 0x6474_E551,
    GnuRelro = 0x6474_E552,
}

bitflags::bitflags! {
    /// Segment permission flags
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct PhFlags: u32 {
        const EXECUTE = 0x1;
        const WRITE   = 0x2;
        const READ    = 0x4;
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf64Phdr {
    pub p_type: u32, // PhType
    pub p_flags: PhFlags,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

impl Elf64Phdr {
    /// Returns true if this is a loadable segment
    pub const fn is_load(&self) -> bool {
        self.p_type == PhType::Load as u32
    }
}
