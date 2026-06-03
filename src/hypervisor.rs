#[cfg(target_os = "linux")]
#[path = "kvm/mod.rs"]
mod hypervisor_backend;

#[cfg(target_os = "illumos")]
#[path = "bhyve/mod.rs"]
mod hypervisor_backend;

#[cfg(not(any(target_os = "linux", target_os = "illumos")))]
compile_error!("This OS is not supported yet!");

#[cfg(target_os = "linux")]
pub use hypervisor_backend::Kvm as NativeHypervisor;

#[cfg(target_os = "illumos")]
pub use hypervisor_backend::Bhyve as NativeHypervisor;
