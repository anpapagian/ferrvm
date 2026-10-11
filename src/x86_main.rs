use std::sync::{Arc, Mutex};

use crate::bus::BusDevice;
use crate::config::Config;
use crate::memory::GuestMemory;
use crate::pci::PciRootBus;
use crate::traits::{Vcpu, Vm};
use crate::virtio::VirtioDevice;
use crate::virtio::pci::{VirtioPciDevice, VirtioPciMmio};

pub const RAM_BASE: u64 = 0;
/// COM1 on the ISA IRQ table.
pub const SERIAL_IRQ: u32 = 4;
/// Upper bound of the virtio-pci BAR window.
const PCI_MMIO_WINDOW_END: u64 = 0xd000_0000;
pub const DEFAULT_CMDLINE: &str = "console=ttyS0,115200 earlyprintk=serial,ttyS0,115200 noapic reboot=t panic=1 oops=panic nomodule rdinit=/sbin/init";

struct NullDevice;
impl BusDevice for NullDevice {
    fn read(&mut self, _: u64, data: &mut [u8]) {
        data.fill(0xff);
    }
    fn write(&mut self, _: u64, _: &[u8]) {}
}

fn register_virtio(
    vm: &dyn Vm,
    memory: &Arc<GuestMemory>,
    root: &mut PciRootBus,
    dispatcher: &mut VirtioPciMmio,
    slot: u8,
    irq: u8,
    device: Box<dyn VirtioDevice>,
) -> Result<(), String> {
    let sink = vm
        .register_irq(u32::from(irq))
        .map_err(|error| format!("Failed to register virtio interrupt {irq}: {error}"))?;
    let transport = Arc::new(Mutex::new(VirtioPciDevice::new(
        Arc::clone(memory),
        sink,
        device,
        irq,
    )));
    root.add_device(slot, 0, transport.clone());
    dispatcher.add_device(transport);
    Ok(())
}

pub fn setup_devices(
    vm: &dyn Vm,
    memory: &Arc<GuestMemory>,
    devices: &crate::Devices,
    config: &Config,
) -> Result<(), String> {
    let root = Arc::new(Mutex::new(PciRootBus::new()));
    root.lock().expect("pci root poisoned").add_device(
        0,
        0,
        Arc::new(Mutex::new(crate::pci::host_bridge())),
    );
    devices
        .pio_bus
        .register(0xcf8, 8, root.clone())
        .map_err(|error| format!("Failed to register PCI config ports: {error}"))?;

    // Linux assigns BARs above guest RAM; dispatch the whole window because
    // individual device BAR addresses are not known until the guest sets them.
    let window_base = memory.size();
    if window_base >= PCI_MMIO_WINDOW_END {
        return Err(format!(
            "Guest RAM ({window_base} bytes) leaves no room for the PCI MMIO window"
        ));
    }
    let dispatcher = Arc::new(Mutex::new(VirtioPciMmio::new(window_base)));
    devices
        .mmio_bus
        .register(
            window_base,
            PCI_MMIO_WINDOW_END - window_base,
            dispatcher.clone(),
        )
        .map_err(|error| format!("Failed to register virtio-pci MMIO window: {error}"))?;

    {
        let mut root = root.lock().expect("pci root poisoned");
        let mut dispatcher = dispatcher.lock().expect("virtio-pci dispatcher poisoned");
        register_virtio(
            vm,
            memory,
            &mut root,
            &mut dispatcher,
            1,
            12,
            Box::new(crate::virtio::rng::VirtioRng::new()),
        )?;
        if let Some(path) = config.disk.as_deref() {
            let block = crate::virtio::blk::VirtioBlk::new(0, path, config.debug)
                .map_err(|error| format!("Failed to open disk image {path}: {error}"))?;
            register_virtio(
                vm,
                memory,
                &mut root,
                &mut dispatcher,
                2,
                14,
                Box::new(block),
            )?;
        }
    }

    devices
        .pio_bus
        .register(0x3f8, 8, devices.serial.clone())
        .map_err(|error| format!("Failed to register COM1: {error}"))?;
    for port in [0x2f8, 0x3e8, 0x2e8] {
        devices
            .pio_bus
            .register(port, 8, Arc::new(Mutex::new(NullDevice)))
            .map_err(|error| format!("Failed to register unused UART at {port:#x}: {error}"))?;
    }
    Ok(())
}

pub fn load_kernel(
    memory: &GuestMemory,
    vcpu: &dyn Vcpu,
    config: &Config,
    cmdline: &str,
) -> Result<(), String> {
    crate::boot::load_kernel(
        memory,
        vcpu,
        &config.kernel,
        cmdline,
        config.initramfs.as_deref(),
    )
    .map_err(|error| format!("Failed to load kernel: {error}"))
}
