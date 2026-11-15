#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

use core::arch::asm;
use core::panic::PanicInfo;

use lazy_static::lazy_static;
use spin::Mutex;
use uart_16550::{Config, Uart16550Tty, backend::PioBackend};
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};

const COM1_BASE: u16 = 0x3F8;

lazy_static! {
    pub static ref SERIAL1: Mutex<Uart16550Tty<PioBackend>> = Mutex::new(unsafe {
        Uart16550Tty::new_port(COM1_BASE, Config::default()).expect("failed to initialize UART")
    });
}

#[doc(hidden)]
pub fn _print(args: ::core::fmt::Arguments) {
    use core::fmt::Write;
    SERIAL1
        .lock()
        .write_fmt(args)
        .expect("Printing to serial failed");
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::_print(format_args!($($arg)*));
    };
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($fmt:expr) => ($crate::print!(concat!($fmt, "\n")));
    ($fmt:expr, $($arg:tt)*) => ($crate::print!(
        concat!($fmt, "\n"), $($arg)*));
}

#[inline]
fn get_cpl() -> u16 {
    let cs: u16;

    unsafe {
        asm!("mov {cs:x}, cs", cs = out(reg) cs);
    }

    cs & 0x3
}

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        idt
    };
}

pub fn init_idt() {
    IDT.load();
}

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    println!("EXCEPTION: BREAKPOINT\n{:#?}", stack_frame);
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    init_idt();

    println!("Hello World from the guest!");
    println!("Running in Ring {}", get_cpl());

    // breakpoint exception
    x86_64::instructions::interrupts::int3();

    println!("Before the empty loop.");

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
