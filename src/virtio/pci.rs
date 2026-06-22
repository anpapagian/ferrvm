//! virtio-pci transport (virtio 1.0).
//!
//! Virtio PCI transport. BAR0 contains the device registers and MSI-X table;
//! interrupts use MSI-X when enabled, with legacy `INTx` as a fallback.

use std::sync::{Arc, Mutex};

use super::{VirtioDevice, Virtqueue};
use crate::bus::BusDevice;
use crate::memory::GuestMemory;
use crate::pci::{ConfigSpace, PciDevice};
use crate::serial::{IrqSink, MsiSink};

// ---- PCI identifiers -------------------------------------------------------
const VIRTIO_PCI_VENDOR: u16 = 0x1af4; // Red Hat / virtio
const VIRTIO_PCI_DEVICE_BASE: u16 = 0x1040; // modern device IDs: 0x1040 + virtio type

// ---- PCI capability chain --------------------------------------------------
const PCI_CAP_ID_VNDR: u8 = 0x09;
const PCI_CAP_ID_MSIX: u8 = 0x11;
const VIRTIO_PCI_CAP_COMMON_CFG: u8 = 1;
const VIRTIO_PCI_CAP_NOTIFY_CFG: u8 = 2;
const VIRTIO_PCI_CAP_ISR_CFG: u8 = 3;
const VIRTIO_PCI_CAP_DEVICE_CFG: u8 = 4;

/// Offsets of the capability structures within PCI configuration space.
const CAP_COMMON: usize = 0x40;
const CAP_NOTIFY: usize = 0x50;
const CAP_ISR: usize = 0x64;
const CAP_DEVICE: usize = 0x74;
const CAP_MSIX: usize = 0x84;

/// `VIRTIO_MSI_NO_VECTOR`: the "no vector assigned" sentinel for the virtio
/// MSI-X configuration fields.
const VIRTIO_MSI_NO_VECTOR: u16 = 0xffff;

/// ISR status bit raised for a used-ring (virtqueue) interrupt.
const ISR_QUEUE: u8 = 0x1;

/// Maximum virtqueue size advertised in `common_cfg.queue_size`. Must be a
/// non-zero power of two; the driver treats a zero value as "queue absent".
const QUEUE_SIZE_MAX: u16 = 256;

// ---- MSI-X capability message control bits ---------------------------------
const MSIX_MSGCTRL_ENABLE: u16 = 0x8000;
const MSIX_MSGCTRL_FUNCTION_MASK: u16 = 0x4000;
/// Per-vector control bit 0: when set the vector is masked.
const MSIX_VECTOR_CTRL_MASK: u32 = 0x1;

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
/// MSI-X table and PBA both live in BAR0 (BIR 0) at these offsets.
const MSIX_TABLE_OFFSET: u64 = 0x0800;
const MSIX_PBA_OFFSET: u64 = 0x0c00;
const MSIX_ENTRY_SIZE: u64 = 16;

/// One MSI-X table entry: 64-bit message address, 32-bit data, and a vector
/// control dword whose bit 0 masks the vector.
#[derive(Clone, Copy)]
struct MsixEntry {
    address: u64,
    data: u32,
    control: u32,
}

impl MsixEntry {
    /// Power-on state: masked, no address/data.
    const fn new() -> Self {
        Self {
            address: 0,
            data: 0,
            control: MSIX_VECTOR_CTRL_MASK,
        }
    }

    const fn masked(&self) -> bool {
        self.control & MSIX_VECTOR_CTRL_MASK != 0
    }
}

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

/// Build the static PCI configuration space for a virtio function with an
/// MSI-X table of `num_vectors` entries.
const fn build_config(virtio_id: u32, irq_line: u8, num_vectors: u16) -> ConfigSpace {
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
        CAP_MSIX as u8, // chain on to the MSI-X capability
        0x10,
        VIRTIO_PCI_CAP_DEVICE_CFG,
        DEVICE_CFG_OFFSET as u32,
        DEVICE_CFG_LEN as u32,
    );

    // MSI-X capability (standard PCI cap 0x11). Message control carries the
    // table size (N-1); the enable and function-mask bits start clear. The
    // table and PBA live in BAR0 (BIR 0) at their fixed offsets.
    cs.set_u8(CAP_MSIX, PCI_CAP_ID_MSIX);
    cs.set_u8(CAP_MSIX + 1, 0x00); // last capability
    cs.set_u16(CAP_MSIX + 2, num_vectors - 1);
    cs.set_u32(CAP_MSIX + 4, MSIX_TABLE_OFFSET as u32); // table offset | BIR 0
    cs.set_u32(CAP_MSIX + 8, MSIX_PBA_OFFSET as u32); // PBA offset | BIR 0

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
    msi_sink: Arc<dyn MsiSink>,
    /// MSI-X table entries (BAR0 `MSIX_TABLE_OFFSET`), one per supported vector.
    msix_table: Vec<MsixEntry>,
    /// Pending bits (BAR0 `MSIX_PBA_OFFSET`): an interrupt arrived while its
    /// vector was masked and is owed once it is unmasked.
    msix_pending: Vec<bool>,
    /// MSI-X vector assigned to each virtqueue (`VIRTIO_MSI_NO_VECTOR` = none).
    queue_msix_vectors: Vec<u16>,
    /// MSI-X vector for configuration-change interrupts.
    config_msix_vector: u16,
}

impl VirtioPciDevice {
    pub fn new(
        mem: Arc<GuestMemory>,
        irq_sink: Arc<dyn IrqSink>,
        msi_sink: Arc<dyn MsiSink>,
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

        // One MSI-X vector per queue plus one for configuration changes.
        let num_vectors = (num_queues + 1) as u16;

        Self {
            config: build_config(virtio_id, irq_line, num_vectors),
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
            msi_sink,
            msix_table: vec![MsixEntry::new(); num_vectors as usize],
            msix_pending: vec![false; num_vectors as usize],
            queue_msix_vectors: vec![VIRTIO_MSI_NO_VECTOR; num_queues],
            config_msix_vector: VIRTIO_MSI_NO_VECTOR,
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

    /// Whether the guest has set the MSI-X enable bit in the capability.
    const fn msix_enabled(&self) -> bool {
        self.config.get_u16(CAP_MSIX + 2) & MSIX_MSGCTRL_ENABLE != 0
    }

    /// Whether the MSI-X function mask (mask-all) bit is set.
    const fn msix_function_masked(&self) -> bool {
        self.config.get_u16(CAP_MSIX + 2) & MSIX_MSGCTRL_FUNCTION_MASK != 0
    }

    /// Raise a used-ring interrupt for `queue_idx`. Uses MSI-X when enabled and
    /// the queue has a vector assigned; otherwise falls back to legacy `INTx`.
    fn raise_queue_interrupt(&mut self, queue_idx: usize) {
        if self.msix_enabled() {
            let vector = self
                .queue_msix_vectors
                .get(queue_idx)
                .copied()
                .unwrap_or(VIRTIO_MSI_NO_VECTOR);
            self.signal_msix(vector);
        } else {
            self.isr_status |= ISR_QUEUE;
            self.irq_sink.set_level(true);
        }
    }

    /// Deliver MSI-X vector `vector`, or record it pending if it is masked.
    fn signal_msix(&mut self, vector: u16) {
        let idx = vector as usize;
        if vector == VIRTIO_MSI_NO_VECTOR || idx >= self.msix_table.len() {
            return;
        }
        if self.msix_function_masked() || self.msix_table[idx].masked() {
            self.msix_pending[idx] = true;
            return;
        }
        let entry = self.msix_table[idx];
        self.msi_sink.signal(entry.address, entry.data);
    }

    /// Deliver any pending MSI-X interrupts whose vector is now unmasked. Called
    /// after a mask bit (per-vector or function-wide) is cleared.
    fn deliver_pending(&mut self) {
        if !self.msix_enabled() || self.msix_function_masked() {
            return;
        }
        for idx in 0..self.msix_table.len() {
            if self.msix_pending[idx] && !self.msix_table[idx].masked() {
                self.msix_pending[idx] = false;
                let entry = self.msix_table[idx];
                self.msi_sink.signal(entry.address, entry.data);
            }
        }
    }

    /// Reset virtio state and vector assignments, preserving the PCI MSI-X table.
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
        self.queue_msix_vectors.fill(VIRTIO_MSI_NO_VECTOR);
        self.config_msix_vector = VIRTIO_MSI_NO_VECTOR;
        self.msix_pending.fill(false);
    }

    /// Read from the MSI-X table at BAR0-relative `rel`.
    fn msix_table_read(&self, rel: u64, data: &mut [u8]) {
        let idx = (rel / MSIX_ENTRY_SIZE) as usize;
        let field = rel % MSIX_ENTRY_SIZE;
        let dword = self.msix_table.get(idx).map_or(0, |e| match field {
            0 => e.address as u32,
            4 => (e.address >> 32) as u32,
            8 => e.data,
            12 => e.control,
            _ => 0,
        });
        put_le(data, u64::from(dword));
    }

    /// Write to the MSI-X table at BAR0-relative `rel`. Clearing a vector's mask
    /// bit delivers any interrupt that arrived while it was masked.
    fn msix_table_write(&mut self, rel: u64, data: &[u8]) {
        let idx = (rel / MSIX_ENTRY_SIZE) as usize;
        let field = rel % MSIX_ENTRY_SIZE;
        let value = get_le(data) as u32;
        if let Some(e) = self.msix_table.get_mut(idx) {
            match field {
                0 => e.address = (e.address & 0xffff_ffff_0000_0000) | u64::from(value),
                4 => e.address = (e.address & 0x0000_0000_ffff_ffff) | (u64::from(value) << 32),
                8 => e.data = value,
                12 => e.control = value,
                _ => {}
            }
        }
        if field == 12 {
            self.deliver_pending();
        }
    }

    /// Read the MSI-X pending-bit array at BAR0-relative `rel` (8-byte qwords).
    fn msix_pba_read(&self, rel: u64, data: &mut [u8]) {
        let base = (rel / 8) * 64;
        let mut qword = 0u64;
        for (idx, &pending) in self.msix_pending.iter().enumerate() {
            let bit = idx as u64;
            if pending && bit >= base && bit < base + 64 {
                qword |= 1 << (bit - base);
            }
        }
        put_le(data, qword);
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
            o if (MSIX_TABLE_OFFSET..MSIX_TABLE_OFFSET + self.msix_table_len()).contains(&o) => {
                self.msix_table_read(o - MSIX_TABLE_OFFSET, data);
            }
            o if (MSIX_PBA_OFFSET..MSIX_PBA_OFFSET + 8).contains(&o) => {
                self.msix_pba_read(o - MSIX_PBA_OFFSET, data);
            }
            _ => data.fill(0),
        }
    }

    /// Length in bytes of the MSI-X table region in BAR0.
    const fn msix_table_len(&self) -> u64 {
        self.msix_table.len() as u64 * MSIX_ENTRY_SIZE
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
                        self.raise_queue_interrupt(queue_idx);
                    }
                }
            }
            o if (DEVICE_CFG_OFFSET..DEVICE_CFG_OFFSET + DEVICE_CFG_LEN).contains(&o) => {
                self.device.write_config(o - DEVICE_CFG_OFFSET, data);
            }
            o if (MSIX_TABLE_OFFSET..MSIX_TABLE_OFFSET + self.msix_table_len()).contains(&o) => {
                self.msix_table_write(o - MSIX_TABLE_OFFSET, data);
            }
            // ISR and the PBA are read-only; anything else is ignored.
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
            // msix_config (0x10): the configuration-change MSI-X vector.
            0x10 => u64::from(self.config_msix_vector),
            // queue_msix_vector (0x1a): the selected queue's MSI-X vector.
            0x1a => u64::from(
                self.queue_msix_vectors
                    .get(self.queue_select as usize)
                    .copied()
                    .unwrap_or(VIRTIO_MSI_NO_VECTOR),
            ),
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
            0x10 => self.config_msix_vector = value as u16,
            0x1a => {
                if let Some(v) = self.queue_msix_vectors.get_mut(sel) {
                    *v = value as u16;
                }
            }
            // Read-only fields are ignored.
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
                // MSI-X message control high byte: only the function-mask
                // (bit 14) and enable (bit 15) bits are writable; the low bits
                // (table size) are read-only.
                o if o == CAP_MSIX + 3 => {
                    let preserved = self.config.get_u8(o) & 0x3f;
                    self.config.set_u8(o, (byte & 0xc0) | preserved);
                }
                _ => {}
            }
        }
        // Re-impose BAR0 type/size encoding: 32-bit, non-prefetchable, 4 KiB.
        let bar = self.config.get_u32(0x10) & 0xffff_f000;
        self.config.set_u32(0x10, bar);
        // Enabling MSI-X or clearing the function mask may release interrupts
        // that were pending while masked.
        self.deliver_pending();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::GuestMemory;
    use crate::serial::NullIrqSink;

    /// Minimal backend: one queue, always asks for a used-ring interrupt.
    struct DummyDevice;
    impl VirtioDevice for DummyDevice {
        fn device_id(&self) -> u32 {
            4 // entropy
        }
        fn device_features(&self) -> u64 {
            super::super::VIRTIO_F_VERSION_1
        }
        fn num_queues(&self) -> usize {
            1
        }
        fn on_notify(&mut self, _idx: usize, _q: &mut Virtqueue, _mem: &GuestMemory) -> bool {
            true
        }
    }

    struct TestMsiSink;
    impl MsiSink for TestMsiSink {
        fn signal(&self, _address: u64, _data: u32) {}
    }

    fn device() -> VirtioPciDevice {
        let mem = Arc::new(GuestMemory::allocate(0x1000).expect("alloc"));
        VirtioPciDevice::new(
            mem,
            Arc::new(NullIrqSink),
            Arc::new(TestMsiSink),
            Box::new(DummyDevice),
            12,
        )
    }

    fn cfg_u8(d: &VirtioPciDevice, off: u64) -> u8 {
        let mut b = [0u8; 1];
        d.config_read(off, &mut b);
        b[0]
    }
    fn cfg_u16(d: &VirtioPciDevice, off: u64) -> u16 {
        let mut b = [0u8; 2];
        d.config_read(off, &mut b);
        u16::from_le_bytes(b)
    }
    fn cfg_u32(d: &VirtioPciDevice, off: u64) -> u32 {
        let mut b = [0u8; 4];
        d.config_read(off, &mut b);
        u32::from_le_bytes(b)
    }

    #[test]
    fn config_advertises_modern_virtio() {
        let d = device();
        assert_eq!(cfg_u16(&d, 0x00), 0x1af4); // virtio vendor
        assert_eq!(cfg_u16(&d, 0x02), 0x1040 + 4); // modern device id for RNG
        assert_eq!(cfg_u8(&d, 0x08), 0x01); // revision 1
        assert_eq!(cfg_u16(&d, 0x06) & 0x0010, 0x0010); // capability list bit
        assert_eq!(cfg_u8(&d, 0x34), CAP_COMMON as u8); // capabilities pointer
        assert_eq!(cfg_u8(&d, 0x3c), 12); // interrupt line
        assert_eq!(cfg_u8(&d, 0x3d), 0x01); // INTA
    }

    #[test]
    fn virtio_capabilities_are_well_formed() {
        let d = device();
        // First cap is the common-cfg structure inside BAR0.
        assert_eq!(cfg_u8(&d, CAP_COMMON as u64), PCI_CAP_ID_VNDR);
        assert_eq!(cfg_u8(&d, CAP_COMMON as u64 + 3), VIRTIO_PCI_CAP_COMMON_CFG);
        assert_eq!(cfg_u8(&d, CAP_DEVICE as u64 + 3), VIRTIO_PCI_CAP_DEVICE_CFG);
    }

    #[test]
    fn bar0_sizing_and_assignment() {
        let mut d = device();
        // All-ones probe reports a 4 KiB, 32-bit, non-prefetchable region.
        d.config_write(0x10, &0xffff_ffffu32.to_le_bytes());
        assert_eq!(cfg_u32(&d, 0x10), 0xffff_f000);
        // A real assignment reads back verbatim (low bits forced to 0).
        d.config_write(0x10, &0x4000_0000u32.to_le_bytes());
        assert_eq!(d.bar_base(), 0x4000_0000);
    }

    #[test]
    fn command_register_gates_memory_space() {
        let mut d = device();
        assert!(!d.mem_enabled());
        d.config_write(0x04, &0x0006u16.to_le_bytes()); // memory + bus master
        assert!(d.mem_enabled());
    }

    #[test]
    fn notify_raises_and_isr_read_clears() {
        let mut d = device();
        let mut num = [0u8; 2];
        d.mmio_read(0x12, &mut num); // common_cfg.num_queues
        assert_eq!(u16::from_le_bytes(num), 1);

        d.mmio_write(NOTIFY_OFFSET, &0u16.to_le_bytes()); // notify queue 0
        let mut isr = [0u8; 1];
        d.mmio_read(ISR_OFFSET, &mut isr);
        assert_eq!(isr[0], ISR_QUEUE);
        // ISR is read-to-clear.
        d.mmio_read(ISR_OFFSET, &mut isr);
        assert_eq!(isr[0], 0);
    }

    #[test]
    fn dispatcher_routes_only_when_enabled() {
        let d = Arc::new(Mutex::new(device()));
        d.lock()
            .unwrap()
            .config_write(0x10, &0x4000_0000u32.to_le_bytes());
        let mut disp = VirtioPciMmio::new(0x4000_0000);
        disp.add_device(d.clone());

        // Memory space still disabled: the window reads back all-ones.
        let mut data = [0u8; 2];
        disp.read(0x12, &mut data);
        assert_eq!(data, [0xff, 0xff]);

        // Enable memory space; the BAR now answers at window base + 0x12.
        d.lock()
            .unwrap()
            .config_write(0x04, &0x0002u16.to_le_bytes());
        disp.read(0x12, &mut data);
        assert_eq!(u16::from_le_bytes(data), 1); // num_queues
    }
}
