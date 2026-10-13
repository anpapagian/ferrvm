# ferrvm

**ferrvm** is a minimal Virtual Machine Monitor (VMM) written in Rust. It has two backends: the Linux KVM API and the illumos bhyve API. The backend is selected automatically at compile time based on the target OS.

[![CI](https://github.com/anpapagian/ferrvm/actions/workflows/rust.yml/badge.svg)](https://github.com/anpapagian/ferrvm/actions/workflows/rust.yml)
[![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![License](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)

## Requirements

- Linux x86_64 with KVM support enabled, or illumos x86_64 with bhyve support enabled
- Rust toolchain (stable)

## Example

ferrvm can boot either an uncompressed ELF kernel (`vmlinux`) or a compressed bzImage (`vmlinuz`).

### Option 1: Boot a bzImage directly

Download a `vmlinuz` from a distribution (e.g. Fedora):

```sh
FEDORA_RELEASE=41
wget "https://download.fedoraproject.org/pub/fedora/linux/releases/${FEDORA_RELEASE}/Everything/x86_64/os/images/pxeboot/vmlinuz"
```

Boot it:

```sh
cargo run -- --kernel ./vmlinuz
```

To boot with an initramfs, pass `--initramfs` in addition to `--kernel`:

```sh
cargo run -- --kernel ./vmlinuz --initramfs ./initramfs.cpio.gz
```

A helper script to build a minimal BusyBox-based initramfs is provided at
[`contrib/build-initramfs.sh`](contrib/build-initramfs.sh). It produces
an `initramfs.cpio.gz` suitable for passing via `--initramfs`.

### Option 2: Boot an uncompressed ELF kernel

Extract a `vmlinux` from the bzImage:

```sh
# Fetch the extraction script from the kernel tree (review before running)
wget -O extract-vmlinux \
  https://raw.githubusercontent.com/torvalds/linux/master/scripts/extract-vmlinux
chmod +x extract-vmlinux
./extract-vmlinux ./vmlinuz > vmlinux
```

Alternatively, any uncompressed (`vmlinux`) x86_64 ELF kernel built with serial console support works.

Boot it:

```sh
cargo run -- --kernel ./vmlinux
```

Expected output: early kernel boot messages over the emulated serial port. Without
an initramfs, the boot terminates at a kernel panic (no rootfs). With an initramfs
supplied via `--initramfs`, the guest proceeds to user space and drops into an
interactive shell on the emulated serial console.

### Options

| Flag           | Description                                        | Default  |
|----------------|----------------------------------------------------|----------|
| `--kernel`     | Path to the kernel image to boot (required)        | —        |
| `--initramfs`  | Path to the initramfs image to boot                | none     |
| `--mem`        | Guest memory in MiB                                | `1024`   |
| `--cmdline`    | Kernel command line                                | built-in |
| `--disk`       | Path to a raw disk image to attach as virtio-blk   | none     |
| `--debug`      | Enable debug mode                                  | off      |

Run `cargo run -- --help` for the full list.

## Console

ferrvm emulates a 16550A UART wired to the guest as COM1 (ttyS0). Its
interrupt line is delivered to the guest through whichever mechanism the
active backend provides — the `KVM_IRQ_LINE` ioctl on Linux, or the
`VM_ISA_ASSERT_IRQ` / `VM_ISA_DEASSERT_IRQ` ioctls on illumos bhyve. The
host terminal is placed into raw mode and a dedicated
reader thread forwards bytes from stdin into the UART's RX FIFO, so
keystrokes reach the guest as-is.

A qemu-style Ctrl-A prefix is reserved for VMM control:

| Key            | Action                               |
|----------------|--------------------------------------|
| `Ctrl-A x`     | quit ferrvm                          |
| `Ctrl-A Ctrl-A`| send a literal `Ctrl-A` to the guest |
| `Ctrl-A ?`     | print the help menu                  |

## Virtio

ferrvm exposes virtio devices over the virtio-pci (virtio 1.0, modern) transport.
A minimal PCI host bridge (Intel 440FX) sits at slot 00:00.0, and each virtio
device appears as a PCI function behind it:

- **virtio-rng** — an entropy source for the guest.
- **virtio-blk** — a block device backed by a raw disk image on the host.

### virtio-blk

Attach a raw disk image with `--disk`:

```sh
# Create a 1 GiB raw image
truncate -s 1G disk.img

cargo run -- --kernel ./vmlinuz --initramfs ./initramfs.cpio.gz --disk ./disk.img
```

The image is exposed to the guest as a virtio-blk device (typically `/dev/vda`).
It is a raw image (no qcow2/format layer); its size determines the reported
capacity, and writes go straight through to the host file.
