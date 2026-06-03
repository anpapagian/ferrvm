/* Sample code for /dev/kvm API
 *
 * Copyright (c) 2015 Intel Corporation
 * Author: Josh Triplett <josh@joshtriplett.org>
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to
 * deal in the Software without restriction, including without limitation the
 * rights to use, copy, modify, merge, publish, distribute, sublicense, and/or
 * sell copies of the Software, and to permit persons to whom the Software is
 * furnished to do so, subject to the following conditions:
 *
 * The above copyright notice and this permission notice shall be included in
 * all copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 * AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
 * FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS
 * IN THE SOFTWARE.
 */
#include <err.h>
#include <fcntl.h>
#include <linux/kvm.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/types.h>

static void print_kvm_regs(const struct kvm_regs *regs)
{
    printf("kvm_regs:\n");
    printf("  rax    = 0x%016llx\n", (unsigned long long)regs->rax);
    printf("  rbx    = 0x%016llx\n", (unsigned long long)regs->rbx);
    printf("  rcx    = 0x%016llx\n", (unsigned long long)regs->rcx);
    printf("  rdx    = 0x%016llx\n", (unsigned long long)regs->rdx);
    printf("  rsi    = 0x%016llx\n", (unsigned long long)regs->rsi);
    printf("  rdi    = 0x%016llx\n", (unsigned long long)regs->rdi);
    printf("  rsp    = 0x%016llx\n", (unsigned long long)regs->rsp);
    printf("  rbp    = 0x%016llx\n", (unsigned long long)regs->rbp);
    printf("  r8     = 0x%016llx\n", (unsigned long long)regs->r8);
    printf("  r9     = 0x%016llx\n", (unsigned long long)regs->r9);
    printf("  r10    = 0x%016llx\n", (unsigned long long)regs->r10);
    printf("  r11    = 0x%016llx\n", (unsigned long long)regs->r11);
    printf("  r12    = 0x%016llx\n", (unsigned long long)regs->r12);
    printf("  r13    = 0x%016llx\n", (unsigned long long)regs->r13);
    printf("  r14    = 0x%016llx\n", (unsigned long long)regs->r14);
    printf("  r15    = 0x%016llx\n", (unsigned long long)regs->r15);
    printf("  rip    = 0x%016llx\n", (unsigned long long)regs->rip);
    printf("  rflags = 0x%016llx\n", (unsigned long long)regs->rflags);
}

static void print_kvm_segment(const char *name, const struct kvm_segment *seg)
{
    printf("  %-3s: base=0x%016llx limit=0x%08x selector=0x%04x type=0x%x\n",
           name,
           (unsigned long long)seg->base,
           seg->limit,
           seg->selector,
           seg->type);
    printf("       present=%u dpl=%u db=%u s=%u l=%u g=%u avl=%u unusable=%u\n",
           seg->present, seg->dpl, seg->db, seg->s,
           seg->l, seg->g, seg->avl, seg->unusable);
}

static void print_kvm_dtable(const char *name, const struct kvm_dtable *dt)
{
    printf("  %s: base=0x%016llx limit=0x%04x\n",
           name, (unsigned long long)dt->base, dt->limit);
}

static void print_kvm_sregs(const struct kvm_sregs *sregs)
{
    size_t i;

    printf("kvm_sregs:\n");
    print_kvm_segment("cs",  &sregs->cs);
    print_kvm_segment("ds",  &sregs->ds);
    print_kvm_segment("es",  &sregs->es);
    print_kvm_segment("fs",  &sregs->fs);
    print_kvm_segment("gs",  &sregs->gs);
    print_kvm_segment("ss",  &sregs->ss);
    print_kvm_segment("tr",  &sregs->tr);
    print_kvm_segment("ldt", &sregs->ldt);
    print_kvm_dtable("gdt", &sregs->gdt);
    print_kvm_dtable("idt", &sregs->idt);
    printf("  cr0       = 0x%016llx\n", (unsigned long long)sregs->cr0);
    printf("  cr2       = 0x%016llx\n", (unsigned long long)sregs->cr2);
    printf("  cr3       = 0x%016llx\n", (unsigned long long)sregs->cr3);
    printf("  cr4       = 0x%016llx\n", (unsigned long long)sregs->cr4);
    printf("  cr8       = 0x%016llx\n", (unsigned long long)sregs->cr8);
    printf("  efer      = 0x%016llx\n", (unsigned long long)sregs->efer);
    printf("  apic_base = 0x%016llx\n", (unsigned long long)sregs->apic_base);
    printf("  interrupt_bitmap =");
    for (i = 0; i < sizeof(sregs->interrupt_bitmap) / sizeof(sregs->interrupt_bitmap[0]); i++)
        printf(" 0x%016llx", (unsigned long long)sregs->interrupt_bitmap[i]);
    printf("\n");
}

static void print_vcpu_state(int vcpufd)
{
    struct kvm_sregs sregs = {0};
    struct kvm_regs regs = {0};
    int ret;

    ret = ioctl(vcpufd, KVM_GET_SREGS, &sregs);
    if (ret == -1)
        err(1, "KVM_GET_SREGS");
    print_kvm_sregs(&sregs);
    printf("\n");

    ret = ioctl(vcpufd, KVM_GET_REGS, &regs);
    if (ret == -1)
        err(1, "KVM_GET_REGS");
    print_kvm_regs(&regs);
    printf("\n");
}

int main(void)
{
    int kvm, vmfd, vcpufd, ret;
    const uint8_t code[] = {
        0xba, 0xf8, 0x03, /* mov $0x3f8, %dx */
        0x00, 0xd8,       /* add %bl, %al */
        0x04, '0',        /* add $'0', %al */
        0xee,             /* out %al, (%dx) */
        0xb0, '\n',       /* mov $'\n', %al */
        0xee,             /* out %al, (%dx) */
        0xf4,             /* hlt */
    };
    uint8_t *mem;
    struct kvm_sregs sregs;
    size_t mmap_size;
    struct kvm_run *run;

    kvm = open("/dev/kvm", O_RDWR | O_CLOEXEC);
    if (kvm == -1)
        err(1, "/dev/kvm");

    /* Make sure we have the stable version of the API */
    ret = ioctl(kvm, KVM_GET_API_VERSION, NULL);
    if (ret == -1)
        err(1, "KVM_GET_API_VERSION");
    if (ret != 12)
        errx(1, "KVM_GET_API_VERSION %d, expected 12", ret);

    vmfd = ioctl(kvm, KVM_CREATE_VM, (unsigned long)0);
    if (vmfd == -1)
        err(1, "KVM_CREATE_VM");

    /* Allocate one aligned page of guest memory to hold the code. */
    mem = mmap(NULL, 0x1000, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANONYMOUS, -1, 0);
    if (!mem)
        err(1, "allocating guest memory");
    memcpy(mem, code, sizeof(code));

    /* Map it to the second page frame (to avoid the real-mode IDT at 0). */
    struct kvm_userspace_memory_region region = {
        .slot = 0,
        .guest_phys_addr = 0x1000,
        .memory_size = 0x1000,
        .userspace_addr = (uint64_t)mem,
    };
    ret = ioctl(vmfd, KVM_SET_USER_MEMORY_REGION, &region);
    if (ret == -1)
        err(1, "KVM_SET_USER_MEMORY_REGION");

    vcpufd = ioctl(vmfd, KVM_CREATE_VCPU, (unsigned long)0);
    if (vcpufd == -1)
        err(1, "KVM_CREATE_VCPU");

    /* Map the shared kvm_run structure and following data. */
    ret = ioctl(kvm, KVM_GET_VCPU_MMAP_SIZE, NULL);
    if (ret == -1)
        err(1, "KVM_GET_VCPU_MMAP_SIZE");
    mmap_size = ret;
    if (mmap_size < sizeof(*run))
        errx(1, "KVM_GET_VCPU_MMAP_SIZE unexpectedly small");
    run = mmap(NULL, mmap_size, PROT_READ | PROT_WRITE, MAP_SHARED, vcpufd, 0);
    if (!run)
        err(1, "mmap vcpu");

    /* Initialize CS to point at 0, via a read-modify-write of sregs. */
    ret = ioctl(vcpufd, KVM_GET_SREGS, &sregs);
    if (ret == -1)
        err(1, "KVM_GET_SREGS");
    sregs.cs.base = 0;
    sregs.cs.selector = 0;
    ret = ioctl(vcpufd, KVM_SET_SREGS, &sregs);
    if (ret == -1)
        err(1, "KVM_SET_SREGS");

    /* Initialize registers: instruction pointer for our code, addends, and
     * initial flags required by x86 architecture. */
    struct kvm_regs regs = {
        .rip = 0x1000,
        .rax = 2,
        .rbx = 2,
        .rflags = 0x2,
    };
    ret = ioctl(vcpufd, KVM_SET_REGS, &regs);
    if (ret == -1)
        err(1, "KVM_SET_REGS");

    print_vcpu_state(vcpufd);

    /* Repeatedly run code and handle VM exits. */
    while (1) {
        ret = ioctl(vcpufd, KVM_RUN, NULL);
        if (ret == -1)
            err(1, "KVM_RUN");
        switch (run->exit_reason) {
        case KVM_EXIT_HLT:
            puts("KVM_EXIT_HLT");
            return 0;
        case KVM_EXIT_IO:
            if (run->io.direction == KVM_EXIT_IO_OUT && run->io.size == 1 && run->io.port == 0x3f8 && run->io.count == 1)
                putchar(*(((char *)run) + run->io.data_offset));
            else
                errx(1, "unhandled KVM_EXIT_IO");
            break;
        case KVM_EXIT_FAIL_ENTRY:
            errx(1, "KVM_EXIT_FAIL_ENTRY: hardware_entry_failure_reason = 0x%llx",
                 (unsigned long long)run->fail_entry.hardware_entry_failure_reason);
        case KVM_EXIT_INTERNAL_ERROR:
            errx(1, "KVM_EXIT_INTERNAL_ERROR: suberror = 0x%x", run->internal.suberror);
        default:
            errx(1, "exit_reason = 0x%x", run->exit_reason);
        }
    }
}
