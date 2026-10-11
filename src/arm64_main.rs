//! ARM64 machine layout, device setup, and direct Linux boot.

use std::borrow::Cow;
use std::fs;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

use ferrvm::printcrln;
use vm_fdt::FdtWriter;

use crate::bus::Bus;
use crate::config::Config;
use crate::memory::GuestMemory;
use crate::traits::{Vcpu, Vm};
use crate::virtio::{VirtioDevice, mmio::VirtioMmioDevice};

pub const GIC_DISTRIBUTOR_BASE: u64 = 0x0800_0000;
pub const GIC_REDISTRIBUTOR_BASE: u64 = 0x0900_0000;
pub const GIC_REDISTRIBUTOR_SIZE: u64 = 0x0200_0000;
pub const UART_BASE: u64 = 0x1000_0000;
pub const UART_SIZE: u64 = 0x1000;
pub const UART_IRQ: u32 = 32;
pub const VIRTIO_MMIO_BASE: u64 = 0x2000_0000;
pub const VIRTIO_MMIO_SIZE: u64 = 0x200;
pub const VIRTIO_RNG_IRQ: u32 = 33;
pub const VIRTIO_BLK_IRQ: u32 = 34;
pub const RAM_BASE: u64 = 0x4000_0000;

pub const SERIAL_IRQ: u32 = UART_IRQ;
pub const DEFAULT_CMDLINE: &str =
    "console=ttyS0,115200 earlycon=uart8250,mmio,0x10000000 panic=1 oops=panic rdinit=/sbin/init";

fn register_virtio(
    vm: &dyn Vm,
    memory: &Arc<GuestMemory>,
    bus: &Bus,
    base: u64,
    irq: u32,
    device: Box<dyn VirtioDevice>,
) -> Result<(), String> {
    let sink = vm
        .register_irq(irq)
        .map_err(|error| format!("Failed to register virtio interrupt {irq}: {error}"))?;
    let transport = Arc::new(Mutex::new(VirtioMmioDevice::new(
        Arc::clone(memory),
        sink,
        device,
    )));
    bus.register(base, VIRTIO_MMIO_SIZE, transport)
        .map_err(|error| format!("Failed to register virtio MMIO at {base:#x}: {error}"))
}

pub fn setup_devices(
    vm: &dyn Vm,
    memory: &Arc<GuestMemory>,
    devices: &crate::Devices,
    config: &Config,
) -> Result<(), String> {
    devices
        .mmio_bus
        .register(UART_BASE, UART_SIZE, devices.serial.clone())
        .map_err(|error| format!("Failed to register UART MMIO: {error}"))?;

    register_virtio(
        vm,
        memory,
        &devices.mmio_bus,
        VIRTIO_MMIO_BASE,
        VIRTIO_RNG_IRQ,
        Box::new(crate::virtio::rng::VirtioRng::new()),
    )?;
    if let Some(path) = config.disk.as_deref() {
        let block = crate::virtio::blk::VirtioBlk::new(0, path, config.debug)
            .map_err(|error| format!("Failed to open disk image {path}: {error}"))?;
        register_virtio(
            vm,
            memory,
            &devices.mmio_bus,
            VIRTIO_MMIO_BASE + VIRTIO_MMIO_SIZE,
            VIRTIO_BLK_IRQ,
            Box::new(block),
        )?;
    }
    Ok(())
}

const FDT_MAX_SIZE: u64 = 0x20_0000;
const ARM64_IMAGE_MAGIC: u32 = 0x644d_5241;
const ZBOOT_MAGIC: &[u8; 4] = b"zimg";

#[derive(Debug)]
struct ImageHeader {
    text_offset: u64,
    image_size: u64,
    flags: u64,
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32, String> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or_else(|| format!("kernel header is truncated at offset {offset:#x}"))?;
    Ok(u32::from_le_bytes(bytes.try_into().expect("four bytes")))
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64, String> {
    let bytes = data
        .get(offset..offset + 8)
        .ok_or_else(|| format!("kernel header is truncated at offset {offset:#x}"))?;
    Ok(u64::from_le_bytes(bytes.try_into().expect("eight bytes")))
}

fn decompress_zboot(data: &[u8]) -> Result<Cow<'_, [u8]>, String> {
    if data.get(4..8) != Some(ZBOOT_MAGIC.as_slice()) {
        return Ok(Cow::Borrowed(data));
    }
    let payload_offset = read_u32(data, 8)? as usize;
    let payload_size = read_u32(data, 12)? as usize;
    let compression = data
        .get(24..32)
        .ok_or("EFI ZBOOT compression field is truncated")?;
    let compression = compression
        .split(|byte| *byte == 0)
        .next()
        .ok_or("EFI ZBOOT compression field is empty")?;
    if compression != b"zstd" {
        return Err(format!(
            "unsupported EFI ZBOOT compression: {}",
            String::from_utf8_lossy(compression)
        ));
    }
    let end = payload_offset
        .checked_add(payload_size)
        .ok_or("EFI ZBOOT payload range overflows")?;
    let payload = data.get(payload_offset..end).ok_or_else(|| {
        format!(
            "EFI ZBOOT payload {payload_offset:#x}..{end:#x} exceeds file size {:#x}",
            data.len()
        )
    })?;
    printcrln!("[boot] Decompressing {payload_size} byte EFI ZBOOT zstd payload");
    zstd::stream::decode_all(Cursor::new(payload))
        .map(Cow::Owned)
        .map_err(|error| format!("failed to decompress EFI ZBOOT payload: {error}"))
}

fn parse_image_header(image: &[u8]) -> Result<ImageHeader, String> {
    if read_u32(image, 0x38)? != ARM64_IMAGE_MAGIC {
        return Err("kernel is not an AArch64 Linux Image (missing ARMx64 magic)".to_string());
    }
    let image_size = read_u64(image, 16)?;
    Ok(ImageHeader {
        text_offset: if image_size == 0 {
            0x8_0000
        } else {
            read_u64(image, 8)?
        },
        image_size: if image_size == 0 {
            image.len() as u64
        } else {
            image_size
        },
        flags: read_u64(image, 24)?,
    })
}

fn add_virtio_node(fdt: &mut FdtWriter, base: u64, irq: u32) -> Result<(), vm_fdt::Error> {
    let node = fdt.begin_node(&format!("virtio_mmio@{base:x}"))?;
    fdt.property_string("compatible", "virtio,mmio")?;
    fdt.property_array_u64("reg", &[base, VIRTIO_MMIO_SIZE])?;
    fdt.property_array_u32("interrupts", &[0, irq - 32, 4])?;
    fdt.property_null("dma-coherent")?;
    fdt.end_node(node)
}

fn create_fdt(
    memory_size: u64,
    cmdline: &str,
    initrd: Option<(u64, u64)>,
    has_disk: bool,
) -> Result<Vec<u8>, vm_fdt::Error> {
    let mut fdt = FdtWriter::new()?;
    let root = fdt.begin_node("")?;
    fdt.property_string("compatible", "linux,dummy-virt")?;
    fdt.property_u32("#address-cells", 2)?;
    fdt.property_u32("#size-cells", 2)?;
    fdt.property_u32("interrupt-parent", 1)?;

    let chosen = fdt.begin_node("chosen")?;
    fdt.property_string("bootargs", cmdline)?;
    fdt.property_string("stdout-path", "/uart@10000000")?;
    if let Some((start, size)) = initrd {
        fdt.property_u64("linux,initrd-start", start)?;
        fdt.property_u64("linux,initrd-end", start + size)?;
    }
    fdt.end_node(chosen)?;

    let memory = fdt.begin_node("memory@40000000")?;
    fdt.property_string("device_type", "memory")?;
    fdt.property_array_u64("reg", &[RAM_BASE, memory_size])?;
    fdt.end_node(memory)?;

    let cpus = fdt.begin_node("cpus")?;
    fdt.property_u32("#address-cells", 1)?;
    fdt.property_u32("#size-cells", 0)?;
    let cpu = fdt.begin_node("cpu@0")?;
    fdt.property_string("device_type", "cpu")?;
    fdt.property_string("compatible", "arm,arm-v8")?;
    fdt.property_u32("reg", 0)?;
    fdt.end_node(cpu)?;
    fdt.end_node(cpus)?;

    let psci = fdt.begin_node("psci")?;
    fdt.property_string("compatible", "arm,psci-0.2")?;
    fdt.property_string("method", "hvc")?;
    fdt.end_node(psci)?;

    let intc = fdt.begin_node("interrupt-controller@8000000")?;
    fdt.property_string("compatible", "arm,gic-v3")?;
    fdt.property_null("interrupt-controller")?;
    fdt.property_u32("#interrupt-cells", 3)?;
    fdt.property_u32("#address-cells", 2)?;
    fdt.property_u32("#size-cells", 2)?;
    fdt.property_null("ranges")?;
    fdt.property_u32("phandle", 1)?;
    fdt.property_array_u64(
        "reg",
        &[
            GIC_DISTRIBUTOR_BASE,
            0x1_0000,
            GIC_REDISTRIBUTOR_BASE,
            GIC_REDISTRIBUTOR_SIZE,
        ],
    )?;
    fdt.end_node(intc)?;
    let timer = fdt.begin_node("timer")?;
    fdt.property_string("compatible", "arm,armv8-timer")?;
    // secure, non-secure physical, virtual, hypervisor PPIs
    fdt.property_array_u32("interrupts", &[1, 13, 8, 1, 14, 8, 1, 11, 8, 1, 10, 8])?;
    fdt.end_node(timer)?;

    let uart = fdt.begin_node("uart@10000000")?;
    fdt.property_string("compatible", "ns16550a")?;
    fdt.property_array_u64("reg", &[UART_BASE, UART_SIZE])?;
    fdt.property_u32("clock-frequency", 1_843_200)?;
    fdt.property_u32("current-speed", 115_200)?;
    fdt.property_array_u32("interrupts", &[0, UART_IRQ - 32, 4])?;
    fdt.end_node(uart)?;

    add_virtio_node(&mut fdt, VIRTIO_MMIO_BASE, VIRTIO_RNG_IRQ)?;
    if has_disk {
        add_virtio_node(
            &mut fdt,
            VIRTIO_MMIO_BASE + VIRTIO_MMIO_SIZE,
            VIRTIO_BLK_IRQ,
        )?;
    }

    fdt.end_node(root)?;
    fdt.finish()
}

fn validate_initrd_range(kernel_address: u64, initrd_end: u64) -> Result<(), String> {
    // Linux requires a 1 GiB-aligned window of up to 32 GiB covering both images.
    let window_base = kernel_address & !((1_u64 << 30) - 1);
    if initrd_end
        .checked_sub(window_base)
        .is_none_or(|span| span > (32_u64 << 30))
    {
        return Err(
            "kernel and initramfs must fit in a 1 GiB-aligned window of up to 32 GiB".to_string(),
        );
    }
    Ok(())
}

pub fn load_kernel(
    memory: &GuestMemory,
    vcpu: &dyn Vcpu,
    config: &Config,
    cmdline: &str,
) -> Result<(), String> {
    let file = fs::read(&config.kernel)
        .map_err(|error| format!("failed to read kernel {}: {error}", config.kernel))?;
    let image = decompress_zboot(&file)?;
    let header = parse_image_header(&image)?;
    if header.flags & 1 != 0 {
        return Err("big-endian AArch64 kernels are not supported".to_string());
    }

    let entry = RAM_BASE
        .checked_add(header.text_offset)
        .ok_or("kernel load address overflows")?;
    let image_size = header.image_size.max(image.len() as u64);
    let kernel_end = entry
        .checked_add(image_size)
        .ok_or("kernel range overflows")?;
    let reserved_size = usize::try_from(image_size).map_err(|_| "kernel image is too large")?;
    memory
        .zero_at(entry, reserved_size)
        .map_err(|error| format!("kernel does not fit in guest memory: {error}"))?;
    memory.write_at(entry, &image)?;

    let initrd = if let Some(path) = config.initramfs.as_deref() {
        let contents =
            fs::read(path).map_err(|error| format!("failed to read initramfs {path}: {error}"))?;
        let start = kernel_end
            .checked_add(0x1f_ffff)
            .ok_or("initramfs address overflows")?
            & !0x1f_ffff;
        let end = start
            .checked_add(contents.len() as u64)
            .ok_or("initramfs range overflows")?;
        validate_initrd_range(entry, end)?;
        memory.write_at(start, &contents)?;
        printcrln!(
            "[boot] Loaded initramfs ({} bytes) at {start:#x}",
            contents.len()
        );
        Some((start, contents.len() as u64))
    } else {
        None
    };

    let fdt = create_fdt(memory.size(), cmdline, initrd, config.disk.is_some())
        .map_err(|error| format!("failed to create device tree: {error}"))?;
    if fdt.len() as u64 > FDT_MAX_SIZE {
        return Err(format!("device tree is too large: {} bytes", fdt.len()));
    }
    let fdt_address = RAM_BASE
        .checked_add(memory.size())
        .and_then(|end| end.checked_sub(FDT_MAX_SIZE))
        .filter(|address| *address >= RAM_BASE)
        .ok_or("guest memory is too small for the device tree")?;
    let payload_end = initrd.map_or(kernel_end, |(start, size)| start + size);
    if payload_end > fdt_address {
        return Err(format!(
            "kernel and initramfs end at {payload_end:#x}, colliding with device tree at {fdt_address:#x}"
        ));
    }
    memory.write_at(fdt_address, &fdt)?;
    vcpu.set_boot_state(entry, fdt_address)
        .map_err(|error| error.to_string())?;
    printcrln!(
        "[boot] Loaded ARM64 Image ({} bytes) at {entry:#x}",
        image.len()
    );
    printcrln!(
        "[boot] Device tree ({} bytes) at {fdt_address:#x}",
        fdt.len(),
    );
    printcrln!("[boot] ARM64 entry={entry:#x}, fdt={fdt_address:#x}");
    Ok(())
}
