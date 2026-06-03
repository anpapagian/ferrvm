#!/bin/bash

# Build a minimal BusyBox initramfs for ferrvm
#
# Usage:
#   ./build-initramfs.sh [output_dir]
#
# Default output_dir is ./ferrvm-initramfs-build

set -euo pipefail

BUSYBOX_VERSION="1.37.0"
BUSYBOX_URL="https://busybox.net/downloads/busybox-${BUSYBOX_VERSION}.tar.bz2"
BUILD_DIR="${1:-$(pwd)/ferrvm-initramfs-build}"
ROOTFS="${BUILD_DIR}/rootfs"
NPROC="$(nproc)"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

info()  { echo -e "${CYAN}[INFO]${NC}  $*"; }
ok()    { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC}  $*"; }
die()   { echo -e "${RED}[ERROR]${NC} $*" >&2; exit 1; }

info "Build directory: ${BUILD_DIR}"
mkdir -p "${BUILD_DIR}"

# Download BusyBox
BUSYBOX_TARBALL="${BUILD_DIR}/busybox-${BUSYBOX_VERSION}.tar.bz2"
BUSYBOX_SRC="${BUILD_DIR}/busybox-${BUSYBOX_VERSION}"

if [ -f "${BUSYBOX_TARBALL}" ]; then
    info "BusyBox tarball already present, skipping download"
else
    info "Downloading BusyBox ${BUSYBOX_VERSION}..."
    wget -q --show-progress -O "${BUSYBOX_TARBALL}" "${BUSYBOX_URL}"
fi

if [ -d "${BUSYBOX_SRC}" ]; then
    info "BusyBox source already extracted, skipping"
else
    info "Extracting BusyBox..."
    tar xjf "${BUSYBOX_TARBALL}" -C "${BUILD_DIR}"
fi

# Build static BusyBox
info "Configuring BusyBox (defconfig + static)..."
cd "${BUSYBOX_SRC}"
make defconfig >/dev/null 2>&1
sed -i 's/# CONFIG_STATIC is not set/CONFIG_STATIC=y/' .config
sed -i 's/CONFIG_TC=y/# CONFIG_TC is not set/' .config

for opt in CONFIG_INIT CONFIG_GETTY CONFIG_MOUNT CONFIG_UMOUNT CONFIG_IP \
           CONFIG_UDHCPC CONFIG_SH_IS_ASH CONFIG_FEATURE_EDITING \
           CONFIG_FEATURE_TAB_COMPLETION CONFIG_VI CONFIG_LESS \
           CONFIG_DMESG CONFIG_REBOOT CONFIG_POWEROFF CONFIG_HALT \
           CONFIG_MKNOD CONFIG_HOSTNAME CONFIG_CAT CONFIG_ECHO \
           CONFIG_GREP CONFIG_FIND CONFIG_LS CONFIG_MKDIR CONFIG_RM \
           CONFIG_CP CONFIG_MV CONFIG_CHMOD CONFIG_CHOWN CONFIG_PS \
           CONFIG_KILL CONFIG_FREE CONFIG_UPTIME CONFIG_PING \
           CONFIG_IFCONFIG CONFIG_ROUTE CONFIG_WGET CONFIG_NC; do
    if grep -q "# ${opt} is not set" .config 2>/dev/null; then
        sed -i "s/# ${opt} is not set/${opt}=y/" .config
    fi
done

info "Building BusyBox with ${NPROC} jobs..."
make -j"${NPROC}" >/dev/null 2>&1 || die "BusyBox build failed."

# Verify static
file busybox | grep -q "statically linked" || warn "BusyBox is NOT statically linked."

ok "BusyBox built successfully"

# Create rootfs skeleton
info "Creating rootfs directory skeleton..."
rm -rf "${ROOTFS}"
mkdir -p "${ROOTFS}"/{bin,sbin,usr/bin,usr/sbin}
mkdir -p "${ROOTFS}"/{etc/init.d,proc,sys,dev,dev/pts,dev/shm}
mkdir -p "${ROOTFS}"/{tmp,run,mnt,root,var/log}

# Install BusyBox into rootfs
make CONFIG_PREFIX="${ROOTFS}" install >/dev/null 2>&1
ok "BusyBox installed into rootfs"
cd "${BUILD_DIR}"

# /etc/inittab
info "Writing /etc/inittab..."
cat > "${ROOTFS}/etc/inittab" << 'EOF'
::sysinit:/etc/init.d/rcS
::respawn:/sbin/getty -n -l /bin/sh -L ttyS0 115200 vt100
::shutdown:/bin/umount -a -r
::shutdown:/sbin/swapoff -a
::ctrlaltdel:/sbin/reboot
EOF

# /etc/init.d/rcS
info "Writing /etc/init.d/rcS..."
cat > "${ROOTFS}/etc/init.d/rcS" << 'RCEOF'
#!/bin/sh

# Mount pseudo-filesystems
mount -t proc     proc     /proc
mount -t sysfs    sysfs    /sys
mount -t devtmpfs devtmpfs /dev
mkdir -p /dev/pts /dev/shm
mount -t devpts   devpts   /dev/pts
mount -t tmpfs    tmpfs    /dev/shm
mount -t tmpfs    tmpfs    /tmp
mount -t tmpfs    tmpfs    /run

# Fallback device nodes (in case devtmpfs is not in the kernel config)
[ -c /dev/console ] || mknod -m 600 /dev/console c 5 1
[ -c /dev/null ]    || mknod -m 666 /dev/null    c 1 3
[ -c /dev/zero ]    || mknod -m 666 /dev/zero    c 1 5
[ -c /dev/ttyS0 ]   || mknod -m 666 /dev/ttyS0   c 4 64
[ -c /dev/tty ]     || mknod -m 666 /dev/tty     c 5 0
[ -c /dev/urandom ] || mknod -m 444 /dev/urandom  c 1 9
[ -c /dev/random ]  || mknod -m 444 /dev/random   c 1 8
[ -c /dev/ptmx ]    || mknod -m 666 /dev/ptmx    c 5 2

# Hostname
hostname ferrvm-guest
echo "ferrvm-guest" > /etc/hostname

# Seed /etc/hosts
cat > /etc/hosts << HOSTS
127.0.0.1   localhost
127.0.1.1   ferrvm-guest
::1         localhost ip6-localhost ip6-loopback
HOSTS

# Bring up loopback
ip link set lo up

# Kernel message level — reduce console noise (errors + warnings only)
echo 4 > /proc/sys/kernel/printk

# Mount debugfs (useful for tracing / perf from inside the guest)
mount -t debugfs debugfs /sys/kernel/debug 2>/dev/null

# Mount securityfs if available
mount -t securityfs securityfs /sys/kernel/security 2>/dev/null

# Seed /dev/urandom from kernel (fast, non-blocking)
[ -f /proc/sys/kernel/random/entropy_avail ] && \
    dd if=/dev/urandom of=/dev/urandom bs=512 count=1 2>/dev/null

echo "============================================"
echo "  ferrvm guest ready"
echo "  kernel:  $(uname -r)"
echo "  cmdline: $(cat /proc/cmdline)"
echo "  memory:  $(free -m 2>/dev/null | awk '/^Mem:/{print $2 " MB"}' || echo 'N/A')"
echo "  press Ctrl+A x to exit..."
echo "============================================"
RCEOF

chmod +x "${ROOTFS}/etc/init.d/rcS"

# /etc/passwd, /etc/group, /etc/shadow
cat > "${ROOTFS}/etc/passwd" << 'EOF'
root:x:0:0:root:/root:/bin/sh
nobody:x:65534:65534:nobody:/nonexistent:/bin/false
EOF

cat > "${ROOTFS}/etc/group" << 'EOF'
root:x:0:
nogroup:x:65534:
EOF

cat > "${ROOTFS}/etc/shadow" << 'EOF'
root::0:0:99999:7:::
nobody:*:0:0:99999:7:::
EOF

chmod 640 "${ROOTFS}/etc/shadow"

# /etc/profile — shell environment
cat > "${ROOTFS}/etc/profile" << 'EOF'
export PATH=/bin:/sbin:/usr/bin:/usr/sbin
export HOME=/root
export TERM=vt100
export HISTFILE=/root/.ash_history
export HISTSIZE=256
export PS1='ferrvm:\w# '
export EDITOR=vi

alias ll='ls -la'
alias la='ls -A'
alias dmesg='dmesg -T 2>/dev/null || dmesg'

cd /root
EOF

# Also source profile from .profile for non-login shell fallback
cat > "${ROOTFS}/root/.profile" << 'EOF'
[ -f /etc/profile ] && . /etc/profile
EOF

# /etc/fstab
cat > "${ROOTFS}/etc/fstab" << 'EOF'
# <device>   <mount>     <type>     <options>   <dump> <fsck>
proc         /proc       proc       defaults    0      0
sysfs        /sys        sysfs      defaults    0      0
devtmpfs     /dev        devtmpfs   defaults    0      0
devpts       /dev/pts    devpts     defaults    0      0
tmpfs        /tmp        tmpfs      defaults    0      0
tmpfs        /run        tmpfs      defaults    0      0
EOF

# Ensure all directories have correct permissions
chmod 1777 "${ROOTFS}/tmp"
chmod 0700 "${ROOTFS}/root"
chmod 0755 "${ROOTFS}/var/log"

# Pack the initramfs
info "Packing initramfs (cpio + gzip)..."
cd "${ROOTFS}"
find . -print0 | cpio --null -o --format=newc --quiet 2>/dev/null | gzip -9 > "${BUILD_DIR}/initramfs.cpio.gz"
cd "${BUILD_DIR}"

# Summary
ok "initramfs built successfully"
ok "Output: ${BUILD_DIR}/initramfs.cpio.gz"
