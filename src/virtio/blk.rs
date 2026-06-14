use super::{VirtioDevice, VirtqDesc, Virtqueue};
use crate::memory::GuestMemory;
use ferrvm::printcrln;
use std::fs::File;
use std::os::unix::fs::FileExt;

const VIRTIO_BLK_T_IN: u32 = 0;
const VIRTIO_BLK_T_OUT: u32 = 1;
const VIRTIO_BLK_T_FLUSH: u32 = 4;

const VIRTIO_BLK_S_OK: u8 = 0;
const VIRTIO_BLK_S_IOERR: u8 = 1;
const VIRTIO_BLK_S_UNSUPP: u8 = 2;

// Feature bit: device supports the flush (cache) command.
const VIRTIO_BLK_F_FLUSH: u64 = 1 << 9;

const SECTOR_SIZE: u64 = 512;

pub struct VirtioBlk {
    disk: File,
    capacity_sectors: u64,
    debug: bool,
}

impl VirtioBlk {
    pub fn new(path: &str, debug: bool) -> std::io::Result<Self> {
        // We should consider using O_DIRECT to avoid double caching
        // both in the host and in guest. This requires proper alignment.
        // We also need to check that does illumos support for direct I/O.
        let disk = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?;
        let capacity_sectors = disk.metadata()?.len() / SECTOR_SIZE;
        Ok(Self {
            disk,
            capacity_sectors,
            debug,
        })
    }

    fn execute(
        &self,
        req_type: u32,
        sector: u64,
        data: &[VirtqDesc],
        mem: &GuestMemory,
        written: &mut u32,
    ) -> u8 {
        if req_type == VIRTIO_BLK_T_FLUSH {
            if self.debug {
                printcrln!("Flush");
            }
            return match self.disk.sync_all() {
                Ok(()) => VIRTIO_BLK_S_OK,
                Err(_) => VIRTIO_BLK_S_IOERR,
            };
        }

        let mut offset = sector * SECTOR_SIZE;

        for desc in data {
            match req_type {
                // read request
                VIRTIO_BLK_T_IN => {
                    let mut buf = vec![0u8; desc.len as usize];
                    if self.debug {
                        printcrln!("Read at offset {}, len {}", offset, desc.len);
                    }
                    if self.disk.read_exact_at(&mut buf, offset).is_err()
                        || mem.write_at(desc.addr, &buf).is_err()
                    {
                        return VIRTIO_BLK_S_IOERR;
                    }
                    *written += desc.len;
                }
                // write request
                VIRTIO_BLK_T_OUT => {
                    let Ok(buf) = mem.read_at(desc.addr, desc.len as usize) else {
                        return VIRTIO_BLK_S_IOERR;
                    };
                    if self.debug {
                        printcrln!("Write at offset {}, len {}", offset, desc.len);
                    }
                    if self.disk.write_all_at(&buf, offset).is_err() {
                        return VIRTIO_BLK_S_IOERR;
                    }
                }
                _ => return VIRTIO_BLK_S_UNSUPP,
            }
            offset += u64::from(desc.len);
        }

        VIRTIO_BLK_S_OK
    }

    fn process_chain(&self, chain: &[VirtqDesc], mem: &GuestMemory) -> u32 {
        if chain.len() < 2 {
            return 0;
        }

        // Request: 16 bytes: type (u32) + reserved (u32) + sector (u64)
        let Ok(hdr) = mem.read_at(chain[0].addr, 16) else {
            return 0;
        };
        let req_type = u32::from_le_bytes(hdr[0..4].try_into().unwrap());
        let sector = u64::from_le_bytes(hdr[8..16].try_into().unwrap());

        let data = &chain[1..chain.len() - 1];
        let mut written = 0u32;
        let status = self.execute(req_type, sector, data, mem, &mut written);

        let status_desc = &chain[chain.len() - 1];
        if mem.write_at(status_desc.addr, &[status]).is_ok() {
            written += 1;
        }

        written
    }
}

impl VirtioDevice for VirtioBlk {
    fn device_id(&self) -> u32 {
        2 // Block device
    }

    fn device_features(&self) -> u64 {
        super::VIRTIO_F_VERSION_1 | VIRTIO_BLK_F_FLUSH
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

    fn read_config(&self, offset: u64, data: &mut [u8]) {
        let cfg = self.capacity_sectors.to_le_bytes();
        for (i, byte) in data.iter_mut().enumerate() {
            let idx = offset as usize + i;
            *byte = if idx < cfg.len() { cfg[idx] } else { 0 };
        }
    }
}
