pub mod blk;
pub mod mmio;
pub mod rng;

pub const VIRTIO_MAGIC: u32 = 0x7472_6976; // "virt"
pub const VIRTIO_VERSION: u32 = 2; // Version 2 (modern)
pub const VIRTIO_VENDOR: u32 = 0x554d_4551; // QEMU vendor ID

pub const VIRTIO_F_VERSION_1: u64 = 1 << 32;

pub const VIRTQ_DESC_F_NEXT: u16 = 1;
pub const VIRTQ_DESC_F_WRITE: u16 = 2;

/// Trait representing a generic Virtio device backend.
pub trait VirtioDevice: Send {
    /// The Virtio Device ID (e.g., 4 for RNG).
    fn device_id(&self) -> u32;

    /// The 64-bit device features.
    fn device_features(&self) -> u64;

    /// The number of virtqueues this device uses.
    fn num_queues(&self) -> usize;

    /// Handle a notification on a specific virtqueue.
    /// Returns true if a used-ring interrupt should be triggered.
    fn on_notify(
        &mut self,
        queue_idx: usize,
        queue: &mut Virtqueue,
        mem: &crate::memory::GuestMemory,
    ) -> bool;

    /// Read from the device-specific configuration space (starts at MMIO offset 0x100).
    fn read_config(&self, _offset: u64, _data: &mut [u8]) {}

    /// Write to the device-specific configuration space (starts at MMIO offset 0x100).
    fn write_config(&mut self, _offset: u64, _data: &[u8]) {}
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct VirtqDesc {
    pub addr: u64,
    pub len: u32,
    pub flags: u16,
    pub next: u16,
}

pub struct Virtqueue {
    pub desc_table: u64,
    pub avail_ring: u64,
    pub used_ring: u64,
    pub num: u16,
    pub last_avail_idx: u16,
    pub ready: bool,
}

impl Virtqueue {
    pub const fn new() -> Self {
        Self {
            desc_table: 0,
            avail_ring: 0,
            used_ring: 0,
            num: 0,
            last_avail_idx: 0,
            ready: false,
        }
    }

    /// Process available descriptors in the queue using the provided device callback.
    /// Returns the number of descriptor chains processed.
    pub fn process<F>(&mut self, mem: &crate::memory::GuestMemory, mut process_chain: F) -> usize
    where
        F: FnMut(&mut [VirtqDesc], &crate::memory::GuestMemory) -> u32,
    {
        if !self.ready || self.num == 0 {
            return 0;
        }

        let Ok(data) = mem.read_at(self.avail_ring + 2, 2) else {
            return 0;
        };
        let avail_idx = u16::from_le_bytes([data[0], data[1]]);

        let mut processed = 0;
        while self.last_avail_idx != avail_idx {
            // Get descriptor chain head index
            let ring_offset = self.avail_ring + 4 + (u64::from(self.last_avail_idx % self.num) * 2);
            let Ok(ring_data) = mem.read_at(ring_offset, 2) else {
                break;
            };
            let head_idx = u16::from_le_bytes([ring_data[0], ring_data[1]]);

            // Collect descriptor chain
            let mut chain = Vec::new();
            let mut desc_idx = head_idx;
            let mut has_next = true;

            while has_next {
                let desc_offset = self.desc_table + (u64::from(desc_idx) * 16);
                let Ok(desc_data) = mem.read_at(desc_offset, 16) else {
                    break;
                };

                let desc = VirtqDesc {
                    addr: u64::from_le_bytes(desc_data[0..8].try_into().unwrap()),
                    len: u32::from_le_bytes(desc_data[8..12].try_into().unwrap()),
                    flags: u16::from_le_bytes(desc_data[12..14].try_into().unwrap()),
                    next: u16::from_le_bytes(desc_data[14..16].try_into().unwrap()),
                };

                chain.push(desc);

                if (desc.flags & VIRTQ_DESC_F_NEXT) != 0 {
                    desc_idx = desc.next;
                } else {
                    has_next = false;
                }

                // Prevent infinite loop by capping chain length
                if chain.len() > usize::from(self.num) {
                    break;
                }
            }

            // Process descriptor chain
            let total_written = process_chain(&mut chain, mem);

            // Write to used ring
            let used_elem_offset =
                self.used_ring + 4 + (u64::from(self.last_avail_idx % self.num) * 8);
            let mut used_elem = [0u8; 8];
            used_elem[0..4].copy_from_slice(&(u32::from(head_idx)).to_le_bytes());
            used_elem[4..8].copy_from_slice(&total_written.to_le_bytes());
            let _ = mem.write_at(used_elem_offset, &used_elem);

            self.last_avail_idx = self.last_avail_idx.wrapping_add(1);
            processed += 1;
        }

        if processed > 0 {
            // Update used index (2 bytes at used_ring + 2)
            let _ = mem.write_at(self.used_ring + 2, &self.last_avail_idx.to_le_bytes());
        }

        processed
    }
}
