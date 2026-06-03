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

/// Default kernel command line if none is provided via CLI.
pub const DEFAULT_CMDLINE: &str = "console=ttyS0,115200 earlyprintk=serial,ttyS0,115200 noapic reboot=t panic=1 oops=panic pci=off nomodule rdinit=/sbin/init virtio_mmio.device=512@0xd0000000:12";

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

    // Register Virtio MMIO RNG device
    let virtio_rng_gsi = 12;
    let virtio_rng_device = Arc::new(Mutex::new(crate::virtio::mmio::VirtioMmioDevice::new(
        guest_mem.clone(),
        Arc::new(crate::serial::NullIrqSink),
        Box::new(crate::virtio::rng::VirtioRng::new()),
    )));

    let virtio_rng_irq_sink = vm
        .register_irq(virtio_rng_gsi)
        .map_err(|e| format!("Failed to register Virtio RNG IRQ: {e}"))?;

    virtio_rng_device
        .lock()
        .expect("virtio rng poisoned")
        .replace_irq_sink(virtio_rng_irq_sink);

    mmio_bus
        .register(0xd000_0000, 512, virtio_rng_device)
        .map_err(|e| format!("Failed to register Virtio MMIO RNG: {e}"))?;

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
