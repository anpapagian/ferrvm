//! virtio-pci transport (virtio 1.0).
//!
//! A `VirtioPciDevice` presents a `VirtioDevice` backend to the guest as a
//! PCI function. Configuration space advertises the IDs, BAR0 and the virtio
//! capability chain; BAR0 is an MMIO region holding the common, ISR, notify and
//! device-specific structures. All BARs are routed through a single
//! `VirtioPciMmio` dispatcher over a fixed window, since the guest assigns
//! BAR addresses at run time. Interrupts use `INTx`.

use std::sync::{Arc, Mutex};

use super::{VirtioDevice, Virtqueue};
use crate::bus::BusDevice;
use crate::memory::GuestMemory;
use crate::pci::{ConfigSpace, PciDevice};
use crate::serial::IrqSink;

// ---- PCI identifiers -------------------------------------------------------
const VIRTIO_PCI_VENDOR: u16 = 0x1af4; // Red Hat / virtio
const VIRTIO_PCI_DEVICE_BASE: u16 = 0x1040; // modern device IDs: 0x1040 + virtio type

// ---- PCI capability chain --------------------------------------------------
const PCI_CAP_ID_VNDR: u8 = 0x09;
const VIRTIO_PCI_CAP_COMMON_CFG: u8 = 1;
const VIRTIO_PCI_CAP_NOTIFY_CFG: u8 = 2;
const VIRTIO_PCI_CAP_ISR_CFG: u8 = 3;
const VIRTIO_PCI_CAP_DEVICE_CFG: u8 = 4;

/// Offsets of the capability structures within PCI configuration space.
const CAP_COMMON: usize = 0x40;
const CAP_NOTIFY: usize = 0x50;
const CAP_ISR: usize = 0x64;
const CAP_DEVICE: usize = 0x74;

/// `VIRTIO_MSI_NO_VECTOR`: reported for every MSI-X vector field since we only
/// implement legacy `INTx`.
const VIRTIO_MSI_NO_VECTOR: u64 = 0xffff;

/// ISR status bit raised for a used-ring (virtqueue) interrupt.
const ISR_QUEUE: u8 = 0x1;

/// Maximum virtqueue size advertised in `common_cfg.queue_size`. Must be a
/// non-zero power of two; the driver treats a zero value as "queue absent".
const QUEUE_SIZE_MAX: u16 = 256;

// ---- BAR0 layout -----------------------------------------------------------
const BAR0_SIZE: u64 = 0x1000;

const COMMON_CFG_OFFSET: u64 = 0x0000;
const COMMON_CFG_LEN: u64 = 0x0040;
const ISR_OFFSET: u64 = 0x0040;
const ISR_LEN: u64 = 0x0010;
const NOTIFY_OFFSET: u64 = 0x0050;
const NOTIFY_LEN: u64 = 0x0010;
const DEVICE_CFG_OFFSET: u64 = 0x0100;
const DEVICE_CFG_LEN: u64 = 0x0100;

/// PCI base class / subclass advertised for a given virtio device type. Linux
/// binds virtio-pci purely on the vendor/device IDs, so this is cosmetic.
const fn pci_class(virtio_id: u32) -> (u8, u8) {
    match virtio_id {
        2 => (0x01, 0x00), // block -> mass storage controller
        _ => (0xff, 0x00), // other / unassigned
    }
}

/// Write one `virtio_pci_cap` header at `at` in configuration space.
const fn write_cap(
    cs: &mut ConfigSpace,
    at: usize,
    next: u8,
    len: u8,
    cfg_type: u8,
    bar_offset: u32,
    bar_len: u32,
) {
    cs.set_u8(at, PCI_CAP_ID_VNDR);
    cs.set_u8(at + 1, next);
    cs.set_u8(at + 2, len);
    cs.set_u8(at + 3, cfg_type);
    cs.set_u8(at + 4, 0); // bar 0
    cs.set_u8(at + 5, 0); // padding
    cs.set_u8(at + 6, 0);
    cs.set_u8(at + 7, 0);
    cs.set_u32(at + 8, bar_offset);
    cs.set_u32(at + 12, bar_len);
}

/// Build the static PCI configuration space for a virtio function.
const fn build_config(virtio_id: u32, irq_line: u8) -> ConfigSpace {
    let mut cs = ConfigSpace::new();
    let (class, subclass) = pci_class(virtio_id);

    cs.set_u16(0x00, VIRTIO_PCI_VENDOR);
    cs.set_u16(0x02, VIRTIO_PCI_DEVICE_BASE + virtio_id as u16);
    cs.set_u16(0x04, 0x0000); // command (writable)
    cs.set_u16(0x06, 0x0010); // status: capabilities list present
    cs.set_u8(0x08, 0x01); // revision: 1 (non-transitional / modern)
    cs.set_u8(0x09, 0x00); // prog IF
    cs.set_u8(0x0a, subclass);
    cs.set_u8(0x0b, class);
    cs.set_u8(0x0e, 0x00); // header type 0
    cs.set_u32(0x10, 0x0000_0000); // BAR0: 32-bit non-prefetchable memory, base 0
    cs.set_u16(0x2c, VIRTIO_PCI_VENDOR); // subsystem vendor
    cs.set_u16(0x2e, virtio_id as u16); // subsystem id
    cs.set_u8(0x34, CAP_COMMON as u8); // capabilities pointer
    cs.set_u8(0x3c, irq_line); // interrupt line (writable)
    cs.set_u8(0x3d, 0x01); // interrupt pin: INTA

    write_cap(
        &mut cs,
        CAP_COMMON,
        CAP_NOTIFY as u8,
        0x10,
        VIRTIO_PCI_CAP_COMMON_CFG,
        COMMON_CFG_OFFSET as u32,
        COMMON_CFG_LEN as u32,
    );
    write_cap(
        &mut cs,
        CAP_NOTIFY,
        CAP_ISR as u8,
        0x14,
        VIRTIO_PCI_CAP_NOTIFY_CFG,
        NOTIFY_OFFSET as u32,
        NOTIFY_LEN as u32,
    );
    cs.set_u32(CAP_NOTIFY + 16, 0); // notify_off_multiplier: 0 (one shared notify reg)
    write_cap(
        &mut cs,
        CAP_ISR,
        CAP_DEVICE as u8,
        0x10,
        VIRTIO_PCI_CAP_ISR_CFG,
        ISR_OFFSET as u32,
        ISR_LEN as u32,
    );
    write_cap(
        &mut cs,
        CAP_DEVICE,
        0x00, // last capability
        0x10,
        VIRTIO_PCI_CAP_DEVICE_CFG,
        DEVICE_CFG_OFFSET as u32,
        DEVICE_CFG_LEN as u32,
    );

    cs
}

/// Pack `value` little-endian into `data` (the access width is `data.len()`).
fn put_le(data: &mut [u8], value: u64) {
    let bytes = value.to_le_bytes();
    for (i, b) in data.iter_mut().enumerate() {
        *b = bytes.get(i).copied().unwrap_or(0);
    }
}

/// Read up to 8 little-endian bytes from `data` into a `u64`.
fn get_le(data: &[u8]) -> u64 {
    let mut value = 0u64;
    for (i, &b) in data.iter().enumerate().take(8) {
        value |= u64::from(b) << (i * 8);
    }
    value
}

/// A single virtio device behind a PCI function (modern transport).
pub struct VirtioPciDevice {
    mem: Arc<GuestMemory>,
    device: Box<dyn VirtioDevice>,
    queues: Vec<Virtqueue>,
    config: ConfigSpace,
    device_feature_select: u32,
    driver_feature_select: u32,
    driver_features: u64,
    status: u8,
    queue_select: u16,
    isr_status: u8,
    irq_sink: Arc<dyn IrqSink>,
}

impl VirtioPciDevice {
    pub fn new(
        mem: Arc<GuestMemory>,
        irq_sink: Arc<dyn IrqSink>,
        device: Box<dyn VirtioDevice>,
        irq_line: u8,
    ) -> Self {
        let virtio_id = device.device_id();
        let num_queues = device.num_queues();
        let mut queues = Vec::with_capacity(num_queues);
        for _ in 0..num_queues {
            let mut queue = Virtqueue::new();
            queue.num = QUEUE_SIZE_MAX; // advertised default; driver may shrink it
            queues.push(queue);
        }

        Self {
            config: build_config(virtio_id, irq_line),
            mem,
            device,
            queues,
            device_feature_select: 0,
            driver_feature_select: 0,
            driver_features: 0,
            status: 0,
            queue_select: 0,
            isr_status: 0,
            irq_sink,
        }
    }

    /// Replace the active IRQ sink (used to swap in the real line at setup).
    pub fn replace_irq_sink(&mut self, sink: Arc<dyn IrqSink>) {
        self.irq_sink = sink;
    }

    /// Current BAR0 base address as programmed by the guest (0 if unassigned).
    pub const fn bar_base(&self) -> u64 {
        (self.config.get_u32(0x10) & 0xffff_f000) as u64
    }

    /// Whether the PCI command register has memory space decoding enabled.
    pub const fn mem_enabled(&self) -> bool {
        self.config.get_u16(0x04) & 0x0002 != 0
    }

    /// Raise a used-ring interrupt: set the ISR queue bit and assert the line.
    fn trigger_interrupt(&mut self) {
        self.isr_status |= ISR_QUEUE;
        self.irq_sink.set_level(true);
    }

    /// Reset all transport state to the power-on defaults.
    fn reset(&mut self) {
        for q in &mut self.queues {
            *q = Virtqueue::new();
            q.num = QUEUE_SIZE_MAX;
        }
        self.device_feature_select = 0;
        self.driver_feature_select = 0;
        self.driver_features = 0;
        self.queue_select = 0;
        self.isr_status = 0;
        self.irq_sink.set_level(false);
    }

    /// Handle a read of BAR0 at `offset`.
    pub fn mmio_read(&mut self, offset: u64, data: &mut [u8]) {
        match offset {
            o if o < COMMON_CFG_OFFSET + COMMON_CFG_LEN => self.common_cfg_read(o, data),
            o if (ISR_OFFSET..ISR_OFFSET + ISR_LEN).contains(&o) => {
                // ISR status is read-to-clear: report it, then drop the line.
                data.fill(0);
                if let Some(first) = data.first_mut() {
                    *first = self.isr_status;
                }
                self.isr_status = 0;
                self.irq_sink.set_level(false);
            }
            o if (NOTIFY_OFFSET..NOTIFY_OFFSET + NOTIFY_LEN).contains(&o) => data.fill(0),
            o if (DEVICE_CFG_OFFSET..DEVICE_CFG_OFFSET + DEVICE_CFG_LEN).contains(&o) => {
                self.device.read_config(o - DEVICE_CFG_OFFSET, data);
            }
            _ => data.fill(0),
        }
    }

    /// Handle a write to BAR0 at `offset`.
    pub fn mmio_write(&mut self, offset: u64, data: &[u8]) {
        match offset {
            o if o < COMMON_CFG_OFFSET + COMMON_CFG_LEN => self.common_cfg_write(o, data),
            o if (NOTIFY_OFFSET..NOTIFY_OFFSET + NOTIFY_LEN).contains(&o) => {
                let queue_idx = get_le(data) as usize;
                if queue_idx < self.queues.len() {
                    let mem = Arc::clone(&self.mem);
                    let queue = &mut self.queues[queue_idx];
                    if self.device.on_notify(queue_idx, queue, &mem) {
                        self.trigger_interrupt();
                    }
                }
            }
            o if (DEVICE_CFG_OFFSET..DEVICE_CFG_OFFSET + DEVICE_CFG_LEN).contains(&o) => {
                self.device.write_config(o - DEVICE_CFG_OFFSET, data);
            }
            // ISR is read-only; anything else is ignored.
            _ => {}
        }
    }

    fn common_cfg_read(&self, offset: u64, data: &mut [u8]) {
        let queue = self.queues.get(self.queue_select as usize);
        let value: u64 = match offset {
            0x00 => u64::from(self.device_feature_select),
            0x04 => {
                let features = self.device.device_features();
                match self.device_feature_select {
                    0 => features & 0xffff_ffff,
                    1 => features >> 32,
                    _ => 0,
                }
            }
            0x08 => u64::from(self.driver_feature_select),
            0x0c => match self.driver_feature_select {
                0 => self.driver_features & 0xffff_ffff,
                1 => self.driver_features >> 32,
                _ => 0,
            },
            // msix_config (0x10) and queue_msix_vector (0x1a): MSI-X unsupported.
            0x10 | 0x1a => VIRTIO_MSI_NO_VECTOR,
            0x12 => self.queues.len() as u64,
            0x14 => u64::from(self.status),
            0x16 => u64::from(self.queue_select),
            0x18 => queue.map_or(0, |q| u64::from(q.num)),
            0x1c => queue.map_or(0, |q| u64::from(q.ready)),
            0x20 => queue.map_or(0, |q| q.desc_table & 0xffff_ffff),
            0x24 => queue.map_or(0, |q| q.desc_table >> 32),
            0x28 => queue.map_or(0, |q| q.avail_ring & 0xffff_ffff),
            0x2c => queue.map_or(0, |q| q.avail_ring >> 32),
            0x30 => queue.map_or(0, |q| q.used_ring & 0xffff_ffff),
            0x34 => queue.map_or(0, |q| q.used_ring >> 32),
            // config_generation (0x15), queue_notify_off (0x1e, multiplier 0)
            // and unknown registers all read as 0.
            _ => 0,
        };
        put_le(data, value);
    }

    fn common_cfg_write(&mut self, offset: u64, data: &[u8]) {
        let value = get_le(data);
        let sel = self.queue_select as usize;
        match offset {
            0x00 => self.device_feature_select = value as u32,
            0x08 => self.driver_feature_select = value as u32,
            0x0c => {
                let half = u64::from(value as u32);
                match self.driver_feature_select {
                    0 => {
                        self.driver_features =
                            (self.driver_features & 0xffff_ffff_0000_0000) | half;
                    }
                    1 => {
                        self.driver_features =
                            (self.driver_features & 0x0000_0000_ffff_ffff) | (half << 32);
                    }
                    _ => {}
                }
            }
            0x14 => {
                self.status = value as u8;
                if self.status == 0 {
                    self.reset();
                }
            }
            0x16 => self.queue_select = value as u16,
            0x18 => {
                if let Some(q) = self.queues.get_mut(sel) {
                    q.num = value as u16;
                }
            }
            0x1c => {
                if let Some(q) = self.queues.get_mut(sel) {
                    q.ready = value != 0;
                }
            }
            0x20 => {
                if let Some(q) = self.queues.get_mut(sel) {
                    q.desc_table = (q.desc_table & 0xffff_ffff_0000_0000) | (value & 0xffff_ffff);
                }
            }
            0x24 => {
                if let Some(q) = self.queues.get_mut(sel) {
                    q.desc_table = (q.desc_table & 0x0000_0000_ffff_ffff) | (value << 32);
                }
            }
            0x28 => {
                if let Some(q) = self.queues.get_mut(sel) {
                    q.avail_ring = (q.avail_ring & 0xffff_ffff_0000_0000) | (value & 0xffff_ffff);
                }
            }
            0x2c => {
                if let Some(q) = self.queues.get_mut(sel) {
                    q.avail_ring = (q.avail_ring & 0x0000_0000_ffff_ffff) | (value << 32);
                }
            }
            0x30 => {
                if let Some(q) = self.queues.get_mut(sel) {
                    q.used_ring = (q.used_ring & 0xffff_ffff_0000_0000) | (value & 0xffff_ffff);
                }
            }
            0x34 => {
                if let Some(q) = self.queues.get_mut(sel) {
                    q.used_ring = (q.used_ring & 0x0000_0000_ffff_ffff) | (value << 32);
                }
            }
            // msix_config (0x10), queue_msix_vector (0x1a) and read-only fields
            // are ignored.
            _ => {}
        }
    }
}

impl PciDevice for VirtioPciDevice {
    fn config_read(&self, offset: u64, data: &mut [u8]) {
        self.config.config_read(offset, data);
    }

    fn config_write(&mut self, offset: u64, data: &[u8]) {
        for (i, &byte) in data.iter().enumerate() {
            let off = offset as usize + i;
            match off {
                // command (0x04-0x05), BAR0 (0x10-0x13) and interrupt line (0x3c)
                // are writable; everything else is read-only.
                0x04 | 0x05 | 0x10..=0x13 | 0x3c => self.config.set_u8(off, byte),
                _ => {}
            }
        }
        // Re-impose BAR0 type/size encoding: 32-bit, non-prefetchable, 4 KiB.
        let bar = self.config.get_u32(0x10) & 0xffff_f000;
        self.config.set_u32(0x10, bar);
    }
}

/// Routes a fixed MMIO window to whichever virtio-pci BAR currently covers the
/// accessed address, so devices need not be re-registered as the guest moves
/// their BARs.
pub struct VirtioPciMmio {
    window_base: u64,
    devices: Vec<Arc<Mutex<VirtioPciDevice>>>,
}

impl VirtioPciMmio {
    pub const fn new(window_base: u64) -> Self {
        Self {
            window_base,
            devices: Vec::new(),
        }
    }

    pub fn add_device(&mut self, device: Arc<Mutex<VirtioPciDevice>>) {
        self.devices.push(device);
    }

    /// Find the device whose enabled BAR0 contains `addr`, returning it along
    /// with the BAR-relative offset.
    fn route(&self, addr: u64) -> Option<(Arc<Mutex<VirtioPciDevice>>, u64)> {
        for device in &self.devices {
            let guard = device.lock().expect("virtio-pci device poisoned");
            if guard.mem_enabled() {
                let base = guard.bar_base();
                if base != 0 && addr >= base && addr < base + BAR0_SIZE {
                    drop(guard);
                    return Some((Arc::clone(device), addr - base));
                }
            }
        }
        None
    }
}

impl BusDevice for VirtioPciMmio {
    fn read(&mut self, offset: u64, data: &mut [u8]) {
        let addr = self.window_base + offset;
        if let Some((device, rel)) = self.route(addr) {
            device
                .lock()
                .expect("virtio-pci device poisoned")
                .mmio_read(rel, data);
        } else {
            data.fill(0xff);
        }
    }

    fn write(&mut self, offset: u64, data: &[u8]) {
        let addr = self.window_base + offset;
        if let Some((device, rel)) = self.route(addr) {
            device
                .lock()
                .expect("virtio-pci device poisoned")
                .mmio_write(rel, data);
        }
    }
}
