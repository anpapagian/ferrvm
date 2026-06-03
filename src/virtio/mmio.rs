use super::{VIRTIO_MAGIC, VIRTIO_VENDOR, VIRTIO_VERSION, VirtioDevice, Virtqueue};
use crate::bus::BusDevice;
use crate::memory::GuestMemory;
use crate::serial::IrqSink;
use std::sync::Arc;

/// Generic Virtio MMIO (Version 2) transport.
pub struct VirtioMmioDevice {
    /// Reference to guest memory
    mem: Arc<GuestMemory>,
    /// The actual device backend (e.g., RNG, Blk, Net)
    device: Box<dyn VirtioDevice>,
    /// Virtqueues
    queues: Vec<Virtqueue>,
    /// Index of currently selected queue
    queue_sel: u32,
    /// Device features selection
    device_features_sel: u32,
    /// Driver features selection
    driver_features_sel: u32,
    /// Acked driver features
    driver_features: u32,
    /// Interrupt status register (bit 0: used ring update, bit 1: config change)
    interrupt_status: u32,
    /// Device status register
    status: u32,
    /// The IRQ line sink to notify the guest
    irq_sink: Arc<dyn IrqSink>,
}

impl VirtioMmioDevice {
    pub fn new(
        mem: Arc<GuestMemory>,
        irq_sink: Arc<dyn IrqSink>,
        device: Box<dyn VirtioDevice>,
    ) -> Self {
        let num_queues = device.num_queues();
        let mut queues = Vec::with_capacity(num_queues);
        for _ in 0..num_queues {
            queues.push(Virtqueue::new());
        }

        Self {
            mem,
            device,
            queues,
            queue_sel: 0,
            device_features_sel: 0,
            driver_features_sel: 0,
            driver_features: 0,
            interrupt_status: 0,
            status: 0,
            irq_sink,
        }
    }

    /// Replace the active IRQ sink.
    pub fn replace_irq_sink(&mut self, sink: Arc<dyn IrqSink>) {
        self.irq_sink = sink;
    }

    /// Trigger a used-ring interrupt to notify the guest.
    fn trigger_interrupt(&mut self) {
        self.interrupt_status |= 1; // used ring update
        self.irq_sink.set_level(true);
    }
}

impl BusDevice for VirtioMmioDevice {
    fn read(&mut self, offset: u64, data: &mut [u8]) {
        if data.len() != 4 && offset < 0x100 {
            return;
        }

        let val = match offset {
            0x00 => VIRTIO_MAGIC,
            0x04 => VIRTIO_VERSION,
            0x08 => self.device.device_id(),
            0x0c => VIRTIO_VENDOR,
            0x10 => {
                let features = self.device.device_features();
                if self.device_features_sel == 0 {
                    features as u32
                } else if self.device_features_sel == 1 {
                    (features >> 32) as u32
                } else {
                    0
                }
            }
            0x34 if (self.queue_sel as usize) < self.queues.len() => 256, // QueueNumMax
            0x44 if (self.queue_sel as usize) < self.queues.len()
                && self.queues[self.queue_sel as usize].ready =>
            {
                1
            } // QueueReady
            0x60 => self.interrupt_status,
            0x70 => self.status,
            0x100..=0xfff => {
                self.device.read_config(offset - 0x100, data);
                return;
            }
            _ => 0,
        };

        if data.len() == 4 {
            data.copy_from_slice(&val.to_le_bytes());
        }
    }

    fn write(&mut self, offset: u64, data: &[u8]) {
        if data.len() != 4 && offset < 0x100 {
            return;
        }

        if offset >= 0x100 {
            self.device.write_config(offset - 0x100, data);
            return;
        }

        let val = u32::from_le_bytes(data.try_into().unwrap());

        match offset {
            0x14 => self.device_features_sel = val,
            0x20 if self.driver_features_sel == 0 => self.driver_features = val,
            0x24 => self.driver_features_sel = val,
            0x30 => self.queue_sel = val,
            0x38 if (self.queue_sel as usize) < self.queues.len() => {
                self.queues[self.queue_sel as usize].num = val as u16;
            }
            0x44 if (self.queue_sel as usize) < self.queues.len() => {
                self.queues[self.queue_sel as usize].ready = val != 0;
            }
            0x50 => {
                // QueueNotify
                let queue_idx = val as usize;
                if queue_idx < self.queues.len() {
                    let mem = Arc::clone(&self.mem);
                    let queue = &mut self.queues[queue_idx];

                    if self.device.on_notify(queue_idx, queue, &mem) {
                        self.trigger_interrupt();
                    }
                }
            }
            0x64 => {
                // InterruptACK
                self.interrupt_status &= !val;
                if self.interrupt_status == 0 {
                    self.irq_sink.set_level(false);
                }
            }
            0x70 => {
                // Status
                self.status = val;
                if val == 0 {
                    // Reset device
                    for q in &mut self.queues {
                        *q = Virtqueue::new();
                    }
                    self.interrupt_status = 0;
                }
            }
            0x80 if (self.queue_sel as usize) < self.queues.len() => {
                let q = &mut self.queues[self.queue_sel as usize];
                q.desc_table = (q.desc_table & 0xFFFF_FFFF_0000_0000) | u64::from(val);
            }
            0x84 if (self.queue_sel as usize) < self.queues.len() => {
                let q = &mut self.queues[self.queue_sel as usize];
                q.desc_table = (q.desc_table & 0x0000_0000_FFFF_FFFF) | (u64::from(val) << 32);
            }
            0x90 if (self.queue_sel as usize) < self.queues.len() => {
                let q = &mut self.queues[self.queue_sel as usize];
                q.avail_ring = (q.avail_ring & 0xFFFF_FFFF_0000_0000) | u64::from(val);
            }
            0x94 if (self.queue_sel as usize) < self.queues.len() => {
                let q = &mut self.queues[self.queue_sel as usize];
                q.avail_ring = (q.avail_ring & 0x0000_0000_FFFF_FFFF) | (u64::from(val) << 32);
            }
            0xa0 if (self.queue_sel as usize) < self.queues.len() => {
                let q = &mut self.queues[self.queue_sel as usize];
                q.used_ring = (q.used_ring & 0xFFFF_FFFF_0000_0000) | u64::from(val);
            }
            0xa4 if (self.queue_sel as usize) < self.queues.len() => {
                let q = &mut self.queues[self.queue_sel as usize];
                q.used_ring = (q.used_ring & 0x0000_0000_FFFF_FFFF) | (u64::from(val) << 32);
            }
            _ => {}
        }
    }
}
