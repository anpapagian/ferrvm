#[macro_export]
macro_rules! printcrln {
    () => {
        print!("\r\n")
    };
    ($($arg:tt)*) => {
        print!("{}\r\n", format_args!($($arg)*))
    };
}
