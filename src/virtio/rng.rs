use super::{VIRTQ_DESC_F_WRITE, VirtioDevice, VirtqDesc, Virtqueue};
use crate::memory::GuestMemory;
use std::io::Read;

pub struct VirtioRng {
    entropy: std::fs::File,
}

impl VirtioRng {
    #[must_use]
    pub fn new() -> Self {
        let entropy = std::fs::File::open("/dev/urandom").expect("Failed to open /dev/urandom");
        Self { entropy }
    }

    /// Process the descriptor chain by writing random bytes into guest-writable buffers.
    fn process_chain(&mut self, chain: &[VirtqDesc], mem: &GuestMemory) -> u32 {
        let mut total_written = 0;

        for desc in chain {
            // The entropy source only writes to guest-writable buffers
            if (desc.flags & VIRTQ_DESC_F_WRITE) != 0 && desc.len > 0 {
                let mut buf = vec![0u8; desc.len as usize];
                if self.entropy.read_exact(&mut buf).is_ok()
                    && mem.write_at(desc.addr, &buf).is_ok()
                {
                    total_written += desc.len;
                }
            }
        }

        total_written
    }
}

impl VirtioDevice for VirtioRng {
    fn device_id(&self) -> u32 {
        4 // Entropy source
    }

    fn device_features(&self) -> u64 {
        super::VIRTIO_F_VERSION_1
    }

    fn num_queues(&self) -> usize {
        1
    }

    fn on_notify(&mut self, queue_idx: usize, queue: &mut Virtqueue, mem: &GuestMemory) -> bool {
        if queue_idx != 0 {
            return false;
        }

        queue.process(mem, |chain, m| self.process_chain(chain, m)) > 0
    }
}
