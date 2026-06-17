use std::{
    path::Path,
    sync::{Arc, Mutex},
};

mod boot;
mod bootparams;
mod bus;
mod config;
mod elf;
mod hypervisor;
mod input;
mod memory;
mod pci;
mod serial;
mod stats;
mod terminal;
mod traits;
mod vcpu;
mod virtio;

use crate::config::Config;
use crate::hypervisor::NativeHypervisor;
use crate::vcpu::dump_state;
use clap::Parser;
use ferrvm::printcrln;
use memory::GuestMemory;

use crate::serial::{NullIrqSink, Serial};
use crate::{
    bus::{Bus, BusDevice},
    traits::Hypervisor,
};

/// COM1 on the ISA IRQ table. With `KVM_CREATE_IRQCHIP`, GSI 4 routes through
/// the 8259 to LAPIC IRQ 0x24 (vector) on x86.
pub const GSI_COM1: u32 = 4;

/// Upper bound of the virtio-pci BAR window, just below the legacy virtio-mmio
/// region at `0xd000_0000`.
pub const PCI_MMIO_WINDOW_END: u64 = 0xd000_0000;

/// Default kernel command line if none is provided via CLI.
pub const DEFAULT_CMDLINE: &str = "console=ttyS0,115200 earlyprintk=serial,ttyS0,115200 noapic reboot=t panic=1 oops=panic nomodule rdinit=/sbin/init";

struct NullDevice;
impl BusDevice for NullDevice {
    fn read(&mut self, _: u64, data: &mut [u8]) {
        data.fill(0xff);
    }
    fn write(&mut self, _: u64, _: &[u8]) {}
}

/// No-op handler for `SIGUSR1`. The signal exists only to kick `KVM_RUN`
/// out of the kernel with `EINTR` so the vCPU loop can observe the shutdown
/// flag. Without a handler installed, the default disposition would
/// terminate the process.
const extern "C" fn sigusr1_noop(_: libc::c_int) {}

fn install_vcpu_kick_handler() -> Result<(), String> {
    // SAFETY: sigaction is a plain C struct; all-zero is a valid initial state.
    let mut sa: libc::sigaction = unsafe { std::mem::zeroed() };
    sa.sa_sigaction = sigusr1_noop as *const () as usize;
    // Intentionally no SA_RESTART: we want KVM_RUN to return EINTR rather
    // than auto-restart through the signal.
    sa.sa_flags = 0;
    // SAFETY: sa_mask points to valid, owned storage of the right type.
    unsafe { libc::sigemptyset(&raw mut sa.sa_mask) };
    // SAFETY: sa is fully initialized and outlives the call; null oldact is allowed.
    let rc = unsafe { libc::sigaction(libc::SIGUSR1, &raw const sa, std::ptr::null_mut()) };
    if rc != 0 {
        return Err(format!(
            "Failed to install SIGUSR1 handler: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn main() -> Result<(), String> {
    let config = Config::parse();

    printcrln!("[hypervisor] Starting Hypervisor initialization");

    let _raw = terminal::RawMode::enter().map_err(|e| format!("Failed to enter raw mode: {e}"))?;
    let shutdown = input::Shutdown::new();

    // Register the vCPU thread (== main thread here) as the kick target
    // BEFORE spawning the stdin reader, so a Ctrl-A x issued during setup
    // can still interrupt us.
    install_vcpu_kick_handler()?;
    // SAFETY: pthread_self has no preconditions and always returns the caller's id.
    shutdown.set_kick_target(unsafe { libc::pthread_self() });

    let hypervisor: Box<dyn Hypervisor> = Box::new(NativeHypervisor::new()?);

    let vm_name = format!("ferrvm-{}", std::process::id());
    let vm = hypervisor
        .create_vm(&vm_name)
        .map_err(|e| format!("Failed to create VM: {e}"))?;
    printcrln!("[vm] Created VM: {vm_name}");

    vm.setup()
        .map_err(|e| format!("Failed to set up VM: {e}"))?;

    // Allocate guest memory.
    let ram_size = config
        .mem
        .checked_mul(1024 * 1024)
        .ok_or_else(|| format!("Memory value is too large: {}", config.mem))?;
    let guest_mem = Arc::new(
        GuestMemory::allocate(ram_size)
            .map_err(|e| format!("Failed to allocate guest memory: {e}"))?,
    );
    printcrln!("[vm] Guest RAM size: {ram_size} bytes");

    // Register guest memory with Hypervisor
    vm.register_memory_region(0, ram_size as u64, guest_mem.host_addr())
        .map_err(|e| format!("Failed to register guest memory with Hypervisor: {e}"))?;
    printcrln!("[vm] Registered guest memory with Hypervisor");

    // Create vCPU
    let mut vcpu = vm
        .create_vcpu(0)
        .map_err(|e| format!("Failed to create vCPU: {e}"))?;
    printcrln!("[vcpu] Created vCPU 0");

    vcpu.setup()
        .map_err(|e| format!("Failed to set up vCPU: {e}"))?;
    printcrln!("[vcpu] vCPU setup complete");

    // Load kernel
    printcrln!("[boot] Loading kernel...");
    printcrln!("[boot] Kernel path: {}", config.kernel);

    if !Path::new(&config.kernel).exists() {
        return Err(format!("Kernel not found at {}", config.kernel));
    }

    let cmdline = config.cmdline.as_ref().map_or_else(
        || {
            printcrln!("[boot] No kernel command line provided, using default:");
            printcrln!("       {}", DEFAULT_CMDLINE);
            DEFAULT_CMDLINE
        },
        |cmdline| {
            printcrln!("[boot] Kernel command line: {}", cmdline);
            cmdline.as_str()
        },
    );

    boot::load_kernel(
        &guest_mem,
        vcpu.as_ref(),
        &config.kernel,
        cmdline,
        config.initramfs.as_deref(),
    )
    .map_err(|e| format!("Failed to load kernel: {e}"))?;

    printcrln!("[boot] Kernel loaded and CPU configured");

    // Debug: dump complete vCPU state before running
    dump_state(vcpu.as_ref()).map_err(|e| format!("Failed to dump vCPU state: {e}"))?;

    printcrln!("[boot] Ready to run vCPU");

    let pio_bus = Bus::new();
    let mmio_bus = Bus::new();

    let pci_root = Arc::new(Mutex::new(crate::pci::PciRootBus::new()));
    pci_root.lock().expect("pci root poisoned").add_device(
        0,
        0,
        Arc::new(Mutex::new(crate::pci::host_bridge())),
    );
    pio_bus
        .register(0xcf8, 8, pci_root.clone())
        .map_err(|e| format!("Failed to register PCI config ports: {e}"))?;

    // virtio-pci BAR window. Without ACPI/firmware, Linux assigns 32-bit BARs
    // just above the top of guest RAM and below the legacy 0xd000_0000 MMIO
    // region. A single dispatcher claims that whole window and routes accesses
    // to whichever device's BAR currently covers them.
    let pci_window_base = ram_size as u64;
    if pci_window_base >= PCI_MMIO_WINDOW_END {
        return Err(format!(
            "Guest RAM ({ram_size} bytes) leaves no room for the PCI MMIO window"
        ));
    }
    let pci_mmio = Arc::new(Mutex::new(crate::virtio::pci::VirtioPciMmio::new(
        pci_window_base,
    )));
    mmio_bus
        .register(
            pci_window_base,
            PCI_MMIO_WINDOW_END - pci_window_base,
            pci_mmio.clone(),
        )
        .map_err(|e| format!("Failed to register virtio-pci MMIO window: {e}"))?;

    // Register Virtio PCI RNG device at slot 00:01.0 (INTx on GSI 12).
    let virtio_rng_gsi = 12;
    let virtio_rng_device = Arc::new(Mutex::new(crate::virtio::pci::VirtioPciDevice::new(
        guest_mem.clone(),
        Arc::new(crate::serial::NullIrqSink),
        Box::new(crate::virtio::rng::VirtioRng::new()),
        virtio_rng_gsi as u8,
    )));

    let virtio_rng_irq_sink = vm
        .register_irq(virtio_rng_gsi)
        .map_err(|e| format!("Failed to register Virtio RNG IRQ: {e}"))?;

    virtio_rng_device
        .lock()
        .expect("virtio rng poisoned")
        .replace_irq_sink(virtio_rng_irq_sink);

    pci_root
        .lock()
        .expect("pci root poisoned")
        .add_device(1, 0, virtio_rng_device.clone());
    pci_mmio
        .lock()
        .expect("virtio-pci dispatcher poisoned")
        .add_device(virtio_rng_device);

    if let Some(disk_path) = &config.disk {
        let virtio_blk_gsi = 14;
        let blk = crate::virtio::blk::VirtioBlk::new(0, disk_path, config.debug)
            .map_err(|e| format!("Failed to open disk image {disk_path}: {e}"))?;

        // Register Virtio PCI block device at slot 00:02.0 (INTx on GSI 14).
        let virtio_blk_device = Arc::new(Mutex::new(crate::virtio::pci::VirtioPciDevice::new(
            guest_mem.clone(),
            Arc::new(crate::serial::NullIrqSink),
            Box::new(blk),
            virtio_blk_gsi as u8,
        )));

        let virtio_blk_irq_sink = vm
            .register_irq(virtio_blk_gsi)
            .map_err(|e| format!("Failed to register Virtio BLK IRQ: {e}"))?;

        virtio_blk_device
            .lock()
            .expect("virtio blk poisoned")
            .replace_irq_sink(virtio_blk_irq_sink);

        pci_root
            .lock()
            .expect("pci root poisoned")
            .add_device(2, 0, virtio_blk_device.clone());
        pci_mmio
            .lock()
            .expect("virtio-pci dispatcher poisoned")
            .add_device(virtio_blk_device);
    }

    let serial = Arc::new(Mutex::new(Serial::new(
        Box::new(std::io::stdout()),
        Arc::new(NullIrqSink),
    )));

    pio_bus
        .register(0x3f8, 8, serial.clone())
        .map_err(|e| format!("Failed to register COM1: {e}"))?; // COM1
    pio_bus
        .register(0x2f8, 8, Arc::new(Mutex::new(NullDevice)))
        .map_err(|e| format!("Failed to register COM2: {e}"))?; // COM2
    pio_bus
        .register(0x3e8, 8, Arc::new(Mutex::new(NullDevice)))
        .map_err(|e| format!("Failed to register COM3: {e}"))?; // COM3
    pio_bus
        .register(0x2e8, 8, Arc::new(Mutex::new(NullDevice)))
        .map_err(|e| format!("Failed to register COM4: {e}"))?; // COM4

    let _reader = input::spawn_stdin_reader(serial.clone(), shutdown.clone());

    crate::serial::register_serial(vm.as_ref(), GSI_COM1, &serial)
        .map_err(|e| format!("Failed to register COM1 serial device with VM: {e}"))?;

    printcrln!("[vcpu] Starting vCPU execution...");

    let _stats = vcpu::run_vcpu(
        vcpu.as_mut(),
        &guest_mem,
        &pio_bus,
        &mmio_bus,
        &shutdown,
        &config,
    )
    .map_err(|e| format!("Failed to run vCPU: {e}"))?;

    printcrln!("[vm] VM execution completed");

    Ok(())
}
