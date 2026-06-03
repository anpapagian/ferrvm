/*
 * 
 * $ uname -a
 * SunOS omnios 5.11 omnios-r151056-1acbca4f5bd i86pc i386 i86pc
 * $ gmake
 * gcc -m64  -o bhyve_api bhyve_api.c
 * $ pfexec ./bhyve_api
 * 4
 * VM_EXITCODE_SUSPENDED: how=3
 */

#include <err.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>
#include <stdint.h>
#include <assert.h>

#define VM_MAX_NAMELEN 128
#define VM_MAX_SEG_NAMELEN 128
#define MB (1024 * 1024UL)

#define VMMCTL_IOC_BASE (('V' << 16) | ('M' << 8))
#define VMM_IOC_BASE (('v' << 16) | ('m' << 8))
#define VMM_LOCK_IOC_BASE (('v' << 16) | ('l' << 8))
#define VMM_CPU_IOC_BASE (('v' << 16) | ('p' << 8))

#define VMM_CREATE_VM (VMMCTL_IOC_BASE | 0x01)
#define VM_RUN (VMM_CPU_IOC_BASE | 0x01)
#define VM_SET_REGISTER (VMM_CPU_IOC_BASE | 0x02)
#define VM_SET_SEGMENT_DESCRIPTOR (VMM_CPU_IOC_BASE | 0x04)
#define VM_ACTIVATE_CPU (VMM_CPU_IOC_BASE | 0x10)
#define VM_RESET_CPU (VMM_CPU_IOC_BASE | 0x16)
#define VM_SET_RUN_STATE (VMM_CPU_IOC_BASE | 0x18)
#define VM_ALLOC_MEMSEG (VMM_LOCK_IOC_BASE | 0x05)
#define VM_MMAP_MEMSEG (VMM_LOCK_IOC_BASE | 0x06)
#define VM_DESTROY_SELF (VMM_IOC_BASE | 0x25)

enum vm_reg_name {
    VM_REG_GUEST_RAX = 0,
    VM_REG_GUEST_RBX = 1,
    VM_REG_GUEST_RIP = 20,
    VM_REG_GUEST_RFLAGS = 21,
    VM_REG_GUEST_CS = 23,
};

#define VRS_RUN (1 << 1)
#define VRK_RESET 0

enum vm_exitcode {
    VM_EXITCODE_INOUT = 0,
    VM_EXITCODE_HLT = 5,
    VM_EXITCODE_SUSPENDED = 14,
};

#define INOUT_IN (1U << 0)
#define VEC_DEFAULT 0
#define VEC_FULFILL_INOUT 3

struct vm_create_req {
    char name[VM_MAX_NAMELEN];
    uint64_t flags;
};

struct vm_activate_cpu {
    int vcpuid;
};

struct vm_vcpu_reset {
    int vcpuid;
    uint32_t kind;
};

struct vm_run_state {
    int vcpuid;
    uint32_t state;
    uint8_t sipi_vector;
    uint8_t _pad[3];
};

struct vm_memseg {
    int segid;
    size_t len;
    char name[VM_MAX_SEG_NAMELEN];
};

struct vm_memmap {
    uint64_t gpa;
    int segid;
    int64_t segoff;
    size_t len;
    int prot;
    int flags;
};

struct seg_desc {
    uint64_t base;
    uint32_t limit;
    uint32_t access;
};

struct vm_seg_desc {
    int cpuid;
    int regnum;
    struct seg_desc desc;
};

struct vm_register {
    int cpuid;
    int regnum;
    uint64_t regval;
};

struct vm_inout {
    uint32_t eax;
    uint16_t port;
    uint8_t bytes;
    uint8_t flags;
    uint8_t addrsize;
    uint8_t segment;
};

struct vm_exit {
    enum vm_exitcode exitcode;
    int inst_length;
    uint64_t rip;
    union {
        struct vm_inout inout;
        struct {
            int how;
            int source;
            uint64_t when;
        } suspended;
    } u;
};

struct vm_entry {
    int cpuid;
    uint32_t cmd;
    void *exit_data;
    union {
        struct vm_inout inout;
    } u;
};

#define VM_LOWMEM 0
#define PROT_ALL (PROT_READ | PROT_WRITE | PROT_EXEC)
#define MAP_GUARD (MAP_PRIVATE | MAP_ANON | MAP_NORESERVE)

int main(void) 
{
    int i, ctl_fd, vm_fd;
    struct vm_create_req create_req;
    const size_t guard_size = 4 * MB;
    const char *vm_name = "myvm";
    const uint8_t code[] = {
        0xba, 0xf8, 0x03, /* mov $0x3f8, %dx */
        0x00, 0xd8,       /* add %bl, %al */
        0x04, '0',        /* add $'0', %al */
        0xee,             /* out %al, (%dx) */
        0xb0, '\n',       /* mov $'\n', %al */
        0xee,             /* out %al, (%dx) */
        0xf4,             /* hlt */
    };
    char vm_path[VM_MAX_NAMELEN + 16];

    
    ctl_fd = open("/dev/vmmctl", O_RDWR | O_EXCL);
    if (ctl_fd < 0) 
        err(1, "open /dev/vmmctl");
    
    memset(&create_req, 0, sizeof(create_req));
    strncpy(create_req.name, vm_name, VM_MAX_NAMELEN);
    
    if (ioctl(ctl_fd, VMM_CREATE_VM, &create_req) < 0) {
        if (errno == EEXIST) {
            snprintf(vm_path, sizeof(vm_path), "/dev/vmm/%s", vm_name);
            vm_fd = open(vm_path, O_RDWR);
            if (vm_fd >= 0) {
                ioctl(vm_fd, VM_DESTROY_SELF, NULL);
                close(vm_fd);
            }

            if (ioctl(ctl_fd, VMM_CREATE_VM, &create_req) < 0) 
                err(1, "VMM_CREATE_VM");
        } else {
            err(1, "VMM_CREATE_VM");
        }
    }
    close(ctl_fd);

    snprintf(vm_path, sizeof(vm_path), "/dev/vmm/%s", vm_name);
    vm_fd = open(vm_path, O_RDWR);
    if (vm_fd < 0)
        err(1, "open %s", vm_path);

    struct vm_activate_cpu ac = { 
        .vcpuid = 0,
    };
    if (ioctl(vm_fd, VM_ACTIVATE_CPU, &ac) < 0) 
        err(1, "VM_ACTIVATE_CPU");

    struct vm_vcpu_reset vr = { 
        .vcpuid = 0, 
        .kind = VRK_RESET, 
    };
    if (ioctl(vm_fd, VM_RESET_CPU, &vr) < 0) 
        err(1, "VM_RESET_CPU");

    struct vm_run_state rs = { 
        .vcpuid = 0, 
        .state = VRS_RUN,
    };
    if (ioctl(vm_fd, VM_SET_RUN_STATE, &rs) < 0)
        err(1, "VM_SET_RUN_STATE");

    size_t mem_size = 2 * MB;
    struct vm_memseg ms = { 
        .segid = VM_LOWMEM, 
        .len = mem_size,
    };
    if (ioctl(vm_fd, VM_ALLOC_MEMSEG, &ms) < 0) 
        err(1, "VM_ALLOC_MEMSEG");

    struct vm_memmap mm = { 
        .gpa = 0, 
        .segid = VM_LOWMEM, 
        .segoff = 0, 
        .len = mem_size, 
        .prot = PROT_ALL 
    };
    if (ioctl(vm_fd, VM_MMAP_MEMSEG, &mm) < 0) 
        err(1, "VM_MMAP_MEMSEG");

    char *base = mmap(NULL, mem_size + 2 * guard_size, PROT_NONE, MAP_GUARD, -1, 0);
    if (base == MAP_FAILED) 
        err(1, "mmap guard");

    char *guest_mem = base + guard_size;
    if (mmap(guest_mem, mem_size, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_FIXED, vm_fd, 0) == MAP_FAILED)
        err(1, "mmap guest memory");

    memcpy(guest_mem + 0x1000, code, sizeof(code));

    struct vm_seg_desc cs_desc = {
        .cpuid = 0, 
        .regnum = VM_REG_GUEST_CS,
        .desc = { 
            .base = 0, 
            .limit = 0xFFFF, 
            .access = 0x0093,
        },
    };
    ioctl(vm_fd, VM_SET_SEGMENT_DESCRIPTOR, &cs_desc);

    struct vm_register regs[] = {
        { 0, VM_REG_GUEST_CS, 0 },
        { 0, VM_REG_GUEST_RIP, 0x1000 },
        { 0, VM_REG_GUEST_RAX, 2 },
        { 0, VM_REG_GUEST_RBX, 2 },
        { 0, VM_REG_GUEST_RFLAGS, 0x2 }
    };
    for (i = 0; i < 5; i++) 
        ioctl(vm_fd, VM_SET_REGISTER, &regs[i]);

    struct vm_exit vmexit;
    struct vm_entry vmentry = { 
        .cpuid = 0, 
        .cmd = VEC_DEFAULT, 
        .exit_data = &vmexit,
    };

    while (1) {
        if (ioctl(vm_fd, VM_RUN, &vmentry) < 0) 
            err(1, "VM_RUN");

        vmentry.cmd = VEC_DEFAULT;
        switch (vmexit.exitcode) {
        case VM_EXITCODE_HLT:
            printf("VM_EXITCODE_HLT\n");
            goto done;
        case VM_EXITCODE_SUSPENDED:
            printf("VM_EXITCODE_SUSPENDED: how=%d\n", vmexit.u.suspended.how);
            goto done;
        case VM_EXITCODE_INOUT:
            if (!(vmexit.u.inout.flags & INOUT_IN) && vmexit.u.inout.port == 0x3f8) {
                printf("%c", (char)vmexit.u.inout.eax);
                fflush(stdout);
                vmentry.cmd = VEC_FULFILL_INOUT;
                vmentry.u.inout = vmexit.u.inout;
            } else {
                errx(1, "unhandled INOUT: port=0x%x", vmexit.u.inout.port);
            }

            break;
        default: 
            errx(1, "unhandled exit: %d", vmexit.exitcode);
        }
    }

done:
    ioctl(vm_fd, VM_DESTROY_SELF, NULL);
    close(vm_fd);
    return 0;
}
