use std::sync::Arc;

use super::{VIRTIO_MAGIC, VIRTIO_VENDOR, VIRTIO_VERSION, VirtioDevice, Virtqueue};
use crate::bus::BusDevice;
use crate::memory::GuestMemory;
use crate::serial::IrqSink;

/// Virtio 1.0 MMIO transport for an individual device.
pub struct VirtioMmioDevice {
    memory: Arc<GuestMemory>,
    device: Box<dyn VirtioDevice>,
    queues: Vec<Virtqueue>,
    queue_sel: u32,
    device_features_sel: u32,
    driver_features_sel: u32,
    driver_features: u64,
    interrupt_status: u32,
    status: u32,
    irq: Arc<dyn IrqSink>,
}

impl VirtioMmioDevice {
    pub fn new(
        memory: Arc<GuestMemory>,
        irq: Arc<dyn IrqSink>,
        device: Box<dyn VirtioDevice>,
    ) -> Self {
        let queues = (0..device.num_queues()).map(|_| Virtqueue::new()).collect();
        Self {
            memory,
            device,
            queues,
            queue_sel: 0,
            device_features_sel: 0,
            driver_features_sel: 0,
            driver_features: 0,
            interrupt_status: 0,
            status: 0,
            irq,
        }
    }

    fn selected_queue_mut(&mut self) -> Option<&mut Virtqueue> {
        self.queues.get_mut(self.queue_sel as usize)
    }

    fn trigger_interrupt(&mut self) {
        self.interrupt_status |= 1;
        self.irq.set_level(true);
    }

    fn reset(&mut self) {
        for queue in &mut self.queues {
            *queue = Virtqueue::new();
        }
        self.queue_sel = 0;
        self.driver_features = 0;
        self.interrupt_status = 0;
        self.irq.set_level(false);
    }
}

impl BusDevice for VirtioMmioDevice {
    fn read(&mut self, offset: u64, data: &mut [u8]) {
        if offset >= 0x100 {
            self.device.read_config(offset - 0x100, data);
            return;
        }
        if data.len() != 4 {
            data.fill(0);
            return;
        }

        let value = match offset {
            0x00 => VIRTIO_MAGIC,
            0x04 => VIRTIO_VERSION,
            0x08 => self.device.device_id(),
            0x0c => VIRTIO_VENDOR,
            0x10 => match self.device_features_sel {
                0 => self.device.device_features() as u32,
                1 => (self.device.device_features() >> 32) as u32,
                _ => 0,
            },
            0x34 if (self.queue_sel as usize) < self.queues.len() => 256,
            0x44 => self
                .queues
                .get(self.queue_sel as usize)
                .map_or(0, |queue| u32::from(queue.ready)),
            0x60 => self.interrupt_status,
            0x70 => self.status,
            _ => 0,
        };
        data.copy_from_slice(&value.to_le_bytes());
    }

    fn write(&mut self, offset: u64, data: &[u8]) {
        if offset >= 0x100 {
            self.device.write_config(offset - 0x100, data);
            return;
        }
        let Ok(bytes) = <[u8; 4]>::try_from(data) else {
            return;
        };
        let value = u32::from_le_bytes(bytes);

        match offset {
            0x14 => self.device_features_sel = value,
            0x20 if self.driver_features_sel < 2 => {
                let shift = self.driver_features_sel * 32;
                let mask = u64::from(u32::MAX) << shift;
                self.driver_features = (self.driver_features & !mask) | (u64::from(value) << shift);
            }
            0x24 => self.driver_features_sel = value,
            0x30 => self.queue_sel = value,
            0x38 => {
                if let Some(queue) = self.selected_queue_mut() {
                    queue.num = value.min(256) as u16;
                }
            }
            0x44 => {
                if let Some(queue) = self.selected_queue_mut() {
                    queue.ready = value != 0;
                }
            }
            0x50 => {
                let index = value as usize;
                if index < self.queues.len() {
                    let memory = Arc::clone(&self.memory);
                    let queue = &mut self.queues[index];
                    if self.device.on_notify(index, queue, &memory) {
                        self.trigger_interrupt();
                    }
                }
            }
            0x64 => {
                self.interrupt_status &= !value;
                if self.interrupt_status == 0 {
                    self.irq.set_level(false);
                }
            }
            0x70 => {
                self.status = value;
                if value == 0 {
                    self.reset();
                }
            }
            0x80 => {
                if let Some(queue) = self.selected_queue_mut() {
                    queue.desc_table =
                        (queue.desc_table & 0xffff_ffff_0000_0000) | u64::from(value);
                }
            }
            0x84 => {
                if let Some(queue) = self.selected_queue_mut() {
                    queue.desc_table =
                        (queue.desc_table & 0x0000_0000_ffff_ffff) | (u64::from(value) << 32);
                }
            }
            0x90 => {
                if let Some(queue) = self.selected_queue_mut() {
                    queue.avail_ring =
                        (queue.avail_ring & 0xffff_ffff_0000_0000) | u64::from(value);
                }
            }
            0x94 => {
                if let Some(queue) = self.selected_queue_mut() {
                    queue.avail_ring =
                        (queue.avail_ring & 0x0000_0000_ffff_ffff) | (u64::from(value) << 32);
                }
            }
            0xa0 => {
                if let Some(queue) = self.selected_queue_mut() {
                    queue.used_ring = (queue.used_ring & 0xffff_ffff_0000_0000) | u64::from(value);
                }
            }
            0xa4 => {
                if let Some(queue) = self.selected_queue_mut() {
                    queue.used_ring =
                        (queue.used_ring & 0x0000_0000_ffff_ffff) | (u64::from(value) << 32);
                }
            }
            _ => {}
        }
    }
}
