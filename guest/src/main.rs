#![no_std]
#![no_main]

use core::arch::asm;
use core::fmt::{self, Write};
use core::panic::PanicInfo;

const COM1_BASE: u16 = 0x3f8;

const REG_THR: u16 = 0;
const REG_IER: u16 = 1;
const REG_FCR: u16 = 2;
const REG_LCR: u16 = 3;
const REG_MCR: u16 = 4;
const REG_LSR: u16 = 5;

const LSR_THRE: u8 = 1 << 5;

const LCR_8N1: u8 = 0x03;
const LCR_DLAB: u8 = 1 << 7;

const FCR_ENABLE: u8 = 1 << 0;
const FCR_CLEAR_RX: u8 = 1 << 1;
const FCR_CLEAR_TX: u8 = 1 << 2;

const MCR_DTR: u8 = 1 << 0;
const MCR_RTS: u8 = 1 << 1;
const MCR_OUT2: u8 = 1 << 3;

const BAUD_DIVISOR_115200: u16 = 1;

#[inline]
unsafe fn outb(port: u16, value: u8) {
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") value);
    }
}

#[inline]
unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    unsafe {
        asm!("in al, dx", out("al") value, in("dx") port);
    }
    value
}

fn serial_init() {
    unsafe {
        outb(COM1_BASE + REG_IER, 0x00);

        outb(COM1_BASE + REG_LCR, LCR_DLAB);
        outb(COM1_BASE + REG_THR, (BAUD_DIVISOR_115200 & 0xff) as u8);
        outb(COM1_BASE + REG_IER, (BAUD_DIVISOR_115200 >> 8) as u8);

        outb(COM1_BASE + REG_LCR, LCR_8N1);

        outb(
            COM1_BASE + REG_FCR,
            FCR_ENABLE | FCR_CLEAR_RX | FCR_CLEAR_TX,
        );

        outb(COM1_BASE + REG_MCR, MCR_DTR | MCR_RTS | MCR_OUT2);
    }
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

struct SerialWriter;

impl Write for SerialWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        serial_print(s);
        Ok(())
    }
}

pub fn print_fmt(args: fmt::Arguments) {
    let _ = SerialWriter.write_fmt(args);
}

macro_rules! print {
    ($($arg:tt)*) => ($crate::print_fmt(format_args!($($arg)*)));
}

macro_rules! println {
    () => (print!("\n"));
    ($($arg:tt)*) => (print!("{}\n", format_args!($($arg)*)));
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    serial_init();

    let val: u32 = 11;

    println!("Hello World from the guest!");
    println!("val = {}", val);

    #[allow(clippy::empty_loop)]
    loop {
        unsafe {
            asm!("hlt");
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    #[allow(clippy::empty_loop)]
    loop {
        unsafe {
            asm!("hlt");
        }
    }
}
