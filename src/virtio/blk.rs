use super::{VirtioDevice, VirtqDesc, Virtqueue};
use crate::memory::GuestMemory;
use ferrvm::printcrln;
use std::fs::File;
use std::os::unix::fs::FileExt;

const VIRTIO_BLK_T_IN: u32 = 0;
const VIRTIO_BLK_T_OUT: u32 = 1;
const VIRTIO_BLK_T_FLUSH: u32 = 4;
const VIRTIO_BLK_T_GET_ID: u32 = 8;

const VIRTIO_BLK_S_OK: u8 = 0;
const VIRTIO_BLK_S_IOERR: u8 = 1;
const VIRTIO_BLK_S_UNSUPP: u8 = 2;

// Feature bit: device reports the maximum number of segments in a request.
const VIRTIO_BLK_F_SEG_MAX: u64 = 1 << 2;
// Feature bit: device supports the flush (cache) command.
const VIRTIO_BLK_F_FLUSH: u64 = 1 << 9;

const SECTOR_SIZE: u64 = 512;

// Maximum number of data segments per request. A descriptor chain holds at
// most QueueNumMax (256, see mmio.rs) descriptors, two of which are reserved
// for the request header and the status byte.
const VIRTIO_BLK_SEG_MAX: u32 = 256 - 2;

const VIRTIO_BLK_ID_BYTES: usize = 20;
const VIRTIO_BLK_ID_PREFIX: &str = "ferrvm-blk";

pub struct VirtioBlk {
    id: u32,
    disk: File,
    capacity_sectors: u64,
    debug: bool,
}

impl VirtioBlk {
    pub fn new(id: u32, path: &str, debug: bool) -> std::io::Result<Self> {
        // We should consider using O_DIRECT to avoid double caching
        // both in the host and in guest. This requires proper alignment.
        // We also need to check that does illumos support for direct I/O.
        let disk = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?;
        let capacity_sectors = disk.metadata()?.len() / SECTOR_SIZE;
        Ok(Self {
            id,
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
                printcrln!("[blk{}] Flush", self.id);
            }
            return match self.disk.sync_all() {
                Ok(()) => VIRTIO_BLK_S_OK,
                Err(_) => VIRTIO_BLK_S_IOERR,
            };
        }

        if req_type == VIRTIO_BLK_T_GET_ID {
            if self.debug {
                printcrln!("[blk{}] Get ID", self.id);
            }
            let Some(desc) = data.first() else {
                return VIRTIO_BLK_S_IOERR;
            };
            // The buffer is VIRTIO_BLK_ID_BYTES long; write the ID and
            // zero-pad the rest.
            let id = format!("{VIRTIO_BLK_ID_PREFIX}{}", self.id);
            let len = (desc.len as usize).min(VIRTIO_BLK_ID_BYTES);
            let mut buf = vec![0u8; len];
            let n = id.len().min(len);
            buf[..n].copy_from_slice(&id.as_bytes()[..n]);
            if mem.write_at(desc.addr, &buf).is_err() {
                return VIRTIO_BLK_S_IOERR;
            }
            *written += len as u32;
            return VIRTIO_BLK_S_OK;
        }

        // Bounds check: the request must lie entirely within the disk's
        // capacity. Without this, a read past EOF fails inconsistently while a
        // write past EOF would silently extend the backing image.
        let total_len: u64 = data.iter().map(|d| u64::from(d.len)).sum();
        let disk_bytes = self.capacity_sectors * SECTOR_SIZE;
        let start = sector.checked_mul(SECTOR_SIZE);
        let end = start.and_then(|s| s.checked_add(total_len));
        match end {
            Some(end) if end <= disk_bytes => {}
            _ => {
                if self.debug {
                    printcrln!(
                        "[blk{}] Out-of-bounds request: sector {}, len {} (capacity {} bytes)",
                        self.id,
                        sector,
                        total_len,
                        disk_bytes
                    );
                }
                return VIRTIO_BLK_S_IOERR;
            }
        }

        let mut offset = sector * SECTOR_SIZE;

        for desc in data {
            match req_type {
                // read request
                VIRTIO_BLK_T_IN => {
                    let mut buf = vec![0u8; desc.len as usize];
                    if self.debug {
                        printcrln!(
                            "[blk{}] Read at offset {}, len {}",
                            self.id,
                            offset,
                            desc.len
                        );
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
                        printcrln!(
                            "[blk{}] Write at offset {}, len {}",
                            self.id,
                            offset,
                            desc.len
                        );
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
        super::VIRTIO_F_VERSION_1 | VIRTIO_BLK_F_FLUSH | VIRTIO_BLK_F_SEG_MAX
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
        // struct virtio_blk_config layout:
        //   le64 capacity;  // offset 0
        //   le32 size_max;  // offset 8
        //   le32 seg_max;   // offset 12
        let mut cfg = [0u8; 16];
        cfg[0..8].copy_from_slice(&self.capacity_sectors.to_le_bytes());
        cfg[12..16].copy_from_slice(&VIRTIO_BLK_SEG_MAX.to_le_bytes());
        for (i, byte) in data.iter_mut().enumerate() {
            let idx = offset as usize + i;
            *byte = if idx < cfg.len() { cfg[idx] } else { 0 };
        }
    }
}
