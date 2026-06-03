/// Statistics about VM exits
#[derive(Debug, Clone, Copy)]
pub struct ExitStats {
    pub io_exits: u64,
    pub mmio_exits: u64,
    pub hlt_exits: u64,
    pub shutdown_exits: u64,
    pub other_exits: u64,
}

impl ExitStats {
    pub const fn new() -> Self {
        Self {
            io_exits: 0,
            mmio_exits: 0,
            hlt_exits: 0,
            shutdown_exits: 0,
            other_exits: 0,
        }
    }

    pub const fn total(&self) -> u64 {
        self.io_exits + self.mmio_exits + self.hlt_exits + self.shutdown_exits + self.other_exits
    }
}

impl Default for ExitStats {
    fn default() -> Self {
        Self::new()
    }
}
