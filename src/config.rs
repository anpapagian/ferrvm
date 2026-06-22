use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about = "ferrvm - A Simple Hypervisor in Rust")]
pub struct Config {
    /// Path to the kernel image to boot
    #[arg(long, required = true)]
    pub kernel: String,

    /// Path to the initramfs image to boot (optional)
    #[arg(long)]
    pub initramfs: Option<String>,

    /// Guest memory in MiB
    #[arg(long, default_value_t = 1024)]
    pub mem: usize,

    /// Number of vCPUs (1–255); values above 1 enable SMP.
    #[arg(long, default_value_t = 1)]
    pub cpus: u32,

    /// Command line arguments to pass to the kernel
    #[arg(long)]
    pub cmdline: Option<String>,

    /// Path to a raw disk image to attach as a virtio-blk device (optional)
    #[arg(long)]
    pub disk: Option<String>,

    /// Enable debug mode
    #[arg(long)]
    pub debug: bool,
}
