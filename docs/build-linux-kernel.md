# Building a custom kernel

## Download a Linux kernel

```bash
wget https://cdn.kernel.org/pub/linux/kernel/v7.x/linux-7.0.9.tar.xz
tar -xvf linux-7.0.9.tar.xz
cd linux-7.0.9
```

## Create default config

```bash
make defconfig
```

## Trim it down

```bash
scripts/config --disable MODULES
scripts/config --enable SERIAL_8250
scripts/config --enable SERIAL_8250_CONSOLE
scripts/config --enable EARLY_PRINTK
scripts/config --enable VIRTIO
scripts/config --enable VIRTIO_MMIO
scripts/config --enable VIRTIO_BLK
scripts/config --enable VIRTIO_NET
scripts/config --enable VIRTIO_CONSOLE
scripts/config --enable HW_RANDOM_VIRTIO
scripts/config --enable EXT4_FS
scripts/config --enable TMPFS
scripts/config --enable DEVTMPFS
scripts/config --enable DEVTMPFS_MOUNT
scripts/config --disable DRM
scripts/config --disable SOUND
scripts/config --disable USB_SUPPORT
scripts/config --disable WIRELESS
scripts/config --disable BLUETOOTH
scripts/config --disable WLAN
```

## Build kernel

```bash
make -j$(nproc) bzImage
```

Now the image is located at `arch/x86/boot/bzImage`.