#!/bin/bash

# Build an AArch64 shell initramfs on macOS from Alpine's minirootfs.
# No cross-compiler is needed because Alpine supplies the guest binaries.

set -euo pipefail

ALPINE_VERSION="${ALPINE_VERSION:-3.24.2}"
ALPINE_BRANCH="${ALPINE_BRANCH:-v3.24}"
BUILD_DIR="${1:-$(pwd)/ferrvm-initramfs-build}"
ARCHIVE_NAME="alpine-minirootfs-${ALPINE_VERSION}-aarch64.tar.gz"
BASE_URL="https://dl-cdn.alpinelinux.org/alpine/${ALPINE_BRANCH}/releases/aarch64"
ARCHIVE="${BUILD_DIR}/${ARCHIVE_NAME}"
OUTPUT="${BUILD_DIR}/initramfs-aarch64.cpio.gz"

export COPYFILE_DISABLE=1

mkdir -p "${BUILD_DIR}"

if [ ! -f "${ARCHIVE}" ]; then
    echo "Downloading ${ARCHIVE_NAME}..."
    curl -fL -o "${ARCHIVE}" "${BASE_URL}/${ARCHIVE_NAME}"
fi

echo "Verifying download..."
curl -fsSL -o "${ARCHIVE}.sha256" "${BASE_URL}/${ARCHIVE_NAME}.sha256"
(
    cd "${BUILD_DIR}"
    shasum -a 256 -c "${ARCHIVE_NAME}.sha256"
)

WORK_DIR=$(mktemp -d "${BUILD_DIR}/initramfs.XXXXXX")
trap 'rm -rf "${WORK_DIR}"' EXIT HUP INT TERM
ROOTFS="${WORK_DIR}/rootfs"
mkdir -p "${ROOTFS}"

echo "Extracting AArch64 root filesystem..."
tar -xzf "${ARCHIVE}" -C "${ROOTFS}"

mkdir -p "${ROOTFS}/etc/init.d" "${ROOTFS}/dev/pts" "${ROOTFS}/proc" "${ROOTFS}/sys"

cat >"${ROOTFS}/etc/inittab" <<'EOF'
::sysinit:/etc/init.d/rcS
::respawn:/sbin/getty -n -l /bin/sh -L ttyS0 115200 vt100
::ctrlaltdel:/sbin/reboot
::shutdown:/bin/umount -a -r
EOF

cat >"${ROOTFS}/etc/init.d/rcS" <<'EOF'
#!/bin/sh

mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev
mkdir -p /dev/pts
mount -t devpts devpts /dev/pts
hostname ferrvm-guest

echo
echo "============================================"
echo "  ferrvm AArch64 guest ready"
echo "  kernel: $(uname -r)"
echo "  press Ctrl-A x to exit"
echo "============================================"
EOF
chmod +x "${ROOTFS}/etc/init.d/rcS"

echo "Packing initramfs..."
(
    cd "${ROOTFS}"
    find . -print | LC_ALL=C sort | cpio -o -H newc
) | gzip -9 >"${WORK_DIR}/initramfs.cpio.gz"
mv "${WORK_DIR}/initramfs.cpio.gz" "${OUTPUT}"

echo "Created ${OUTPUT}"
