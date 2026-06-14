#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;

const COM1_BASE: u16 = 0x3f8;
const REG_THR: u16 = 0;
const REG_LSR: u16 = 5;
const LSR_THRE: u8 = 1 << 5;

#[inline]
unsafe fn outb(port: u16, value: u8) {
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
    }
}

#[inline]
unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    unsafe {
        asm!("in al, dx", out("al") value, in("dx") port, options(nomem, nostack, preserves_flags));
    }
    value
}

fn serial_putb(byte: u8) {
    while unsafe { inb(COM1_BASE + REG_LSR) } & LSR_THRE == 0 {
        core::hint::spin_loop();
    }
    unsafe { outb(COM1_BASE + REG_THR, byte) };
}

fn serial_print(s: &str) {
    for &byte in s.as_bytes() {
        if byte == b'\n' {
            serial_putb(b'\r');
        }
        serial_putb(byte);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    serial_print("** Hello World from the guest! **\n");

    #[allow(clippy::empty_loop)]
    loop {}
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
