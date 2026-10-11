mod bus;
mod config;
mod input;
mod memory;
mod serial;
mod stats;
mod terminal;
mod traits;
mod vcpu;
mod virtio;

#[cfg(target_arch = "x86_64")]
mod boot;
#[cfg(target_arch = "x86_64")]
mod bootparams;
#[cfg(target_arch = "x86_64")]
mod elf;
mod hypervisor;
#[cfg(target_arch = "x86_64")]
mod pci;

#[cfg(target_arch = "aarch64")]
#[path = "arm64_main.rs"]
mod machine;
#[cfg(target_arch = "x86_64")]
#[path = "x86_main.rs"]
mod machine;

use std::path::Path;
use std::sync::{Arc, Mutex};

use clap::Parser;
use ferrvm::printcrln;

use crate::bus::Bus;
use crate::config::Config;
use crate::hypervisor::NativeHypervisor;
use crate::memory::GuestMemory;
use crate::serial::Serial;
use crate::traits::Hypervisor;

struct Devices {
    pio_bus: Bus,
    mmio_bus: Bus,
    serial: Arc<Mutex<Serial>>,
}

fn main() -> Result<(), String> {
    let config = Config::parse();
    if !Path::new(&config.kernel).exists() {
        return Err(format!("Kernel not found at {}", config.kernel));
    }

    let _raw =
        terminal::RawMode::enter().map_err(|error| format!("Failed to enter raw mode: {error}"))?;
    let shutdown = input::Shutdown::new();
    input::install_vcpu_kick_handler()?;
    // SAFETY: pthread_self has no preconditions; this thread owns the vCPU.
    shutdown.set_kick_target(unsafe { libc::pthread_self() });

    #[cfg(target_arch = "x86_64")]
    let hypervisor = NativeHypervisor::new()?;
    #[cfg(target_arch = "aarch64")]
    let hypervisor = NativeHypervisor::new();
    let vm_name = format!("ferrvm-{}", std::process::id());
    let vm = Hypervisor::create_vm(&hypervisor, &vm_name)
        .map_err(|error| format!("Failed to create VM: {error}"))?;
    vm.setup()
        .map_err(|error| format!("Failed to set up VM: {error}"))?;

    let ram_size = config
        .mem
        .checked_mul(1024 * 1024)
        .ok_or_else(|| format!("Memory value is too large: {}", config.mem))?;
    let memory = Arc::new(GuestMemory::allocate_at(machine::RAM_BASE, ram_size)?);
    vm.register_memory_region(memory.base(), memory.size(), memory.host_addr())
        .map_err(|error| format!("Failed to map guest memory: {error}"))?;
    printcrln!(
        "[vm] Mapped {} MiB of RAM at {:#x}",
        config.mem,
        memory.base()
    );

    let mut vcpu = vm
        .create_vcpu(0)
        .map_err(|error| format!("Failed to create vCPU: {error}"))?;
    vcpu.setup()
        .map_err(|error| format!("Failed to set up vCPU: {error}"))?;

    let serial_irq = vm
        .register_irq(machine::SERIAL_IRQ)
        .map_err(|error| format!("Failed to register UART interrupt: {error}"))?;
    let devices = Devices {
        pio_bus: Bus::new(),
        mmio_bus: Bus::new(),
        serial: Arc::new(Mutex::new(Serial::new(
            Box::new(std::io::stdout()),
            serial_irq,
        ))),
    };
    machine::setup_devices(vm.as_ref(), &memory, &devices, &config)?;

    let cmdline = config
        .cmdline
        .as_deref()
        .unwrap_or(machine::DEFAULT_CMDLINE);
    printcrln!("[boot] Kernel command line: {cmdline}");
    machine::load_kernel(&memory, vcpu.as_ref(), &config, cmdline)?;
    vcpu::dump_state(vcpu.as_ref())?;

    let reader = input::spawn_stdin_reader(devices.serial, shutdown.clone());
    printcrln!("[vcpu] Starting vCPU execution...");
    let result = vcpu::run_vcpu(
        vcpu.as_mut(),
        &memory,
        &devices.pio_bus,
        &devices.mmio_bus,
        &shutdown,
        &config,
    );
    // Stop the input thread before its interrupt sinks can outlive the VM.
    shutdown.request();
    reader.join().map_err(|_| "stdin reader panicked")?;
    result?;
    Ok(())
}

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
compile_error!("This CPU architecture is not supported yet!");
