pub const EDD_MBR_SIG_MAX: usize = 16; /* max number of signatures to store */
pub const EDDMAXNR: usize = 6; /* number of edd_info structs starting at EDDBUF */

// <https://www.kernel.org/doc/html/latest/x86/boot.html>
pub const KERNEL_TYPE_OF_LOADER: u8 = 0xff;
pub const KERNEL_BOOT_FLAG_MAGIC_NUMBER: u16 = 0xaa55;
pub const KERNEL_HDR_MAGIC_NUMBER: u32 = 0x5372_6448;
pub const KERNEL_MIN_ALIGNMENT_BYTES: u32 = 0x0100_0000;

// Value taken from <https://elixir.bootlin.com/linux/v5.10.68/source/arch/x86/include/uapi/asm/e820.h#L31>
pub const E820_RAM: u32 = 1;
pub const E820_RESERVED: u32 = 2;
pub const E820_MAX_ENTRIES_ZEROPAGE: usize = 128;

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct ScreenInfo {
    pub orig_x: u8,             /* 0x00 */
    pub orig_y: u8,             /* 0x01 */
    pub ext_mem_k: u16,         /* 0x02 */
    pub orig_video_page: u16,   /* 0x04 */
    pub orig_video_mode: u8,    /* 0x06 */
    pub orig_video_cols: u8,    /* 0x07 */
    pub flags: u8,              /* 0x08 */
    pub unused2: u8,            /* 0x09 */
    pub orig_video_ega_bx: u16, /* 0x0a */
    pub unused3: u16,           /* 0x0c */
    pub orig_video_lines: u8,   /* 0x0e */
    pub orig_video_isvga: u8,   /* 0x0f */
    pub orig_video_points: u16, /* 0x10 */

    /* VESA graphic mode -- linear frame buffer */
    pub lfb_width: u16,  /* 0x12 */
    pub lfb_height: u16, /* 0x14 */
    pub lfb_depth: u16,  /* 0x16 */
    pub lfb_base: u32,   /* 0x18 */
    pub lfb_size: u32,   /* 0x1c */
    pub cl_magic: u16,
    pub cl_offset: u16,       /* 0x20 */
    pub lfb_linelength: u16,  /* 0x24 */
    pub red_size: u8,         /* 0x26 */
    pub red_pos: u8,          /* 0x27 */
    pub green_size: u8,       /* 0x28 */
    pub green_pos: u8,        /* 0x29 */
    pub blue_size: u8,        /* 0x2a */
    pub blue_pos: u8,         /* 0x2b */
    pub rsvd_size: u8,        /* 0x2c */
    pub rsvd_pos: u8,         /* 0x2d */
    pub vesapm_seg: u16,      /* 0x2e */
    pub vesapm_off: u16,      /* 0x30 */
    pub pages: u16,           /* 0x32 */
    pub vesa_attributes: u16, /* 0x34 */
    pub capabilities: u32,    /* 0x36 */
    pub ext_lfb_base: u32,    /* 0x3a */
    pub _reserved: [u8; 2],   /* 0x3e */
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ApmBiosInfo {
    pub version: u16,
    pub cseg: u16,
    pub offset: u32,
    pub cseg_16: u16,
    pub dseg: u16,
    pub flags: u16,
    pub cseg_len: u16,
    pub cseg_16_len: u16,
    pub dseg_len: u16,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct IstInfo {
    pub signature: u32,
    pub command: u32,
    pub event: u32,
    pub perf_level: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SysDescTable {
    pub length: u16,
    pub table: [u8; 14],
}

/* Gleaned from OFW's set-parameters in cpu/x86/pc/linux.fth */
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct OlpcOfwHeader {
    pub ofw_magic: u32, /* OFW signature */
    pub ofw_version: u32,
    pub cif_handler: u32, /* callback into OFW */
    pub irq_desc_table: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct EdidInfo {
    pub dummy: [u8; 128],
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct EfiInfo {
    pub efi_loader_signature: u32,
    pub efi_systab: u32,
    pub efi_memdesc_size: u32,
    pub efi_memdesc_version: u32,
    pub efi_memmap: u32,
    pub efi_memmap_size: u32,
    pub efi_systab_hi: u32,
    pub efi_memmap_hi: u32,
}

/// <https://elixir.bootlin.com/linux/v6.12.6/source/arch/x86/include/uapi/asm/bootparam.h#L38>
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct SetupHeader {
    pub setup_sects: u8,
    pub root_flags: u16,
    pub syssize: u32,
    pub ram_size: u16,
    pub vid_mode: u16,
    pub root_dev: u16,
    pub boot_flag: u16,
    pub jump: u16,
    pub header: u32,
    pub version: u16,
    pub realmode_swtch: u32,
    pub start_sys_seg: u16,
    pub kernel_version: u16,
    pub type_of_loader: u8,
    pub loadflags: u8,
    pub setup_move_size: u16,
    pub code32_start: u32,
    pub ramdisk_image: u32,
    pub ramdisk_size: u32,
    pub bootsect_kludge: u32,
    pub heap_end_ptr: u16,
    pub ext_loader_ver: u8,
    pub ext_loader_type: u8,
    pub cmd_line_ptr: u32,
    pub initrd_addr_max: u32,
    pub kernel_alignment: u32,
    pub relocatable_kernel: u8,
    pub min_alignment: u8,
    pub xloadflags: u16,
    pub cmdline_size: u32,
    pub hardware_subarch: u32,
    pub hardware_subarch_data: u64,
    pub payload_offset: u32,
    pub payload_length: u32,
    pub setup_data: u64,
    pub pref_address: u64,
    pub init_size: u32,
    pub handover_offset: u32,
    pub kernel_info_offset: u32,
}

/// <https://elixir.bootlin.com/linux/v6.12.6/source/arch/x86/include/asm/e820/types.h#L55>
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct E820Entry {
    pub addr: u64,
    pub size: u64,
    pub entry_type: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct InterfacePathIsa {
    pub base_address: u16,
    pub reserved1: u16,
    pub reserved2: u32,
}

/* pcix is same as pci */
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct InterfacePathPci {
    pub bus: u8,
    pub slot: u8,
    pub function: u8,
    pub channel: u8,
    pub reserved: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct InterfacePathIbnd {
    pub reserved: u64,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct InterfacePathXprs {
    pub reserved: u64,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct InterfacePathHtpt {
    pub reserved: u64,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct InterfacePathUnknown {
    pub reserved: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union InterfacePath {
    isa: InterfacePathIsa,
    pci: InterfacePathPci,
    ibnd: InterfacePathIbnd,
    xprs: InterfacePathXprs,
    htpt: InterfacePathHtpt,
    unknown: InterfacePathUnknown,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct DevicePathAta {
    pub device: u8,
    pub reserved1: u8,
    pub reserved2: u16,
    pub reserved3: u32,
    pub reserved4: u64,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct DevicePathAtapi {
    pub device: u8,
    pub lun: u8,
    pub reserved1: u8,
    pub reserved2: u8,
    pub reserved3: u32,
    pub reserved4: u64,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct DevicePathUnknown {
    pub reserved1: u64,
    pub reserved2: u64,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct DevicePathScsi {
    pub id: u16,
    pub lun: u64,
    pub reserved1: u16,
    pub reserved2: u32,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct DevicePathUsb {
    pub serial_number: u64,
    pub reserved: u64,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct DevicePathI1394 {
    pub eui: u64,
    pub reserved: u64,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct DevicePathFibre {
    pub wwid: u64,
    pub lun: u64,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct DevicePathI2o {
    pub identity_tag: u64,
    pub reserved: u64,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct DevicePathRaid {
    pub array_number: u32,
    pub reserved1: u32,
    pub reserved2: u64,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct DevicePathSata {
    pub device: u8,
    pub reserved1: u8,
    pub reserved2: u16,
    pub reserved3: u32,
    pub reserved4: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union DevicePath {
    pub ata: DevicePathAta,
    pub atapi: DevicePathAtapi,
    pub scsi: DevicePathScsi,
    pub usb: DevicePathUsb,
    pub i1394: DevicePathI1394,
    pub fibre: DevicePathFibre,
    pub i2o: DevicePathI2o,
    pub raid: DevicePathRaid,
    pub sata: DevicePathSata,
    pub unknown: DevicePathUnknown,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct EddDeviceParams {
    pub length: u16,
    pub info_flags: u16,
    pub num_default_cylinders: u32,
    pub num_default_heads: u32,
    pub sectors_per_track: u32,
    pub number_of_sectors: u64,
    pub bytes_per_sector: u16,
    pub dpte_ptr: u32,               /* 0xFFFFFFFF for our purposes */
    pub key: u16,                    /* = 0xBEDD */
    pub device_path_info_length: u8, /* = 44 */
    pub reserved2: u8,
    pub reserved3: u16,
    pub host_bus_type: [u8; 4],
    pub interface_type: [u8; 8],
    pub interface_path: InterfacePath,
    pub device_path: DevicePath,
    pub reserved4: u8,
    pub checksum: u8,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct EddInfo {
    pub device: u8,
    pub version: u8,
    pub interface_support: u16,
    pub legacy_max_cylinder: u16,
    pub legacy_max_head: u8,
    pub legacy_sectors_per_track: u8,
    pub params: EddDeviceParams,
}

/// <https://elixir.bootlin.com/linux/v6.12.6/source/arch/x86/include/uapi/asm/bootparam.h#L116>
#[repr(C, packed)]
pub struct BootParams {
    pub screen_info: ScreenInfo,        /* 0x000 */
    pub apm_bios_info: ApmBiosInfo,     /* 0x040 */
    pub _pad2: [u8; 4],                 /* 0x054 */
    pub tboot_addr: u64,                /* 0x058 */
    pub ist_info: IstInfo,              /* 0x060 */
    pub acpi_rsdp_addr: u64,            /* 0x070 */
    pub _pad3: [u8; 8],                 /* 0x078 */
    pub hd0_info: [u8; 16],             /* 0x080 obsolete! */
    pub hd1_info: [u8; 16],             /* 0x090 obsolete! */
    pub sys_desc_table: SysDescTable,   /* 0x0a0 obsolete! */
    pub olpc_ofw_header: OlpcOfwHeader, /* 0x0b0 */
    pub ext_ramdisk_image: u32,         /* 0x0c0 */
    pub ext_ramdisk_size: u32,          /* 0x0c4 */
    pub ext_cmd_line_ptr: u32,          /* 0x0c8 */
    pub _pad4: [u8; 112],               /* 0x0cc */
    pub cc_blob_address: u32,           /* 0x13c */
    pub edid_info: EdidInfo,            /* 0x140 */
    pub efi_info: EfiInfo,              /* 0x1c0 */
    pub alt_mem_k: u32,                 /* 0x1e0 */
    pub scratch: u32,                   /* 0x1e4 Scratch field! */
    pub e820_entries: u8,               /* 0x1e8 */
    pub eddbuf_entries: u8,             /* 0x1e9 */
    pub edd_mbr_sig_buf_entries: u8,    /* 0x1ea */
    pub kbd_status: u8,                 /* 0x1eb */
    pub secure_boot: u8,                /* 0x1ec */
    pub _pad5: [u8; 2],                 /* 0x1ed */
    /*
     * The sentinel is set to a nonzero value (0xff) in header.S.
     *
     * A bootloader is supposed to only take setup_header and put
     * it into a clean boot_params buffer. If it turns out that
     * it is clumsy or too generous with the buffer, it most
     * probably will pick up the sentinel variable too. The fact
     * that this variable then is still 0xff will let kernel
     * know that some variables in boot_params are invalid and
     * kernel should zero out certain portions of boot_params.
     */
    pub sentinel: u8,     /* 0x1ef */
    pub _pad6: [u8; 1],   /* 0x1f0 */
    pub hdr: SetupHeader, /* 0x1f1 setup header */
    pub _pad7: [u8; 0x290 - 0x1f1 - size_of::<SetupHeader>()],
    pub edd_mbr_sig_buffer: [u32; EDD_MBR_SIG_MAX], /* 0x290 */
    pub e820_table: [E820Entry; E820_MAX_ENTRIES_ZEROPAGE], /* 0x2d0 */
    pub _pad8: [u8; 48],                            /* 0xcd0 */
    pub eddbuf: [EddInfo; EDDMAXNR],                /* 0xd00 */
    pub _pad9: [u8; 276],                           /* 0xeec */
}

// Compile-time size assertion. Ensure that BootParams is exactly 4096 bytes (4 KiB),
// which is the size expected by the Linux kernel and the x86 boot protocol.
const _: () = assert!(
    std::mem::size_of::<BootParams>() == 4096,
    "BootParams must be exactly 4096 bytes (4 KiB)"
);

impl BootParams {
    pub const fn new() -> Self {
        // SAFETY: BootParams is plain old data, all-zero is a valid bit pattern.
        unsafe { std::mem::zeroed() }
    }
}
