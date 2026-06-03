//! Host terminal raw mode.
use std::io;
use std::os::unix::io::RawFd;
use std::sync::Once;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, Ordering};

use ferrvm::printcrln;

/// RAII guard. While alive, stdin is in raw mode. On Drop (including during
/// panic unwind), the original termios is restored.
pub struct RawMode {
    fd: RawFd,
    original: libc::termios,
    active: bool,
}

impl RawMode {
    /// Enter raw mode on stdin (fd 0). Returns Ok(guard) if successful, or
    /// Ok(inert guard) if stdin isn't a tty — in that case the "raw mode"
    /// is a no-op.
    pub fn enter() -> io::Result<Self> {
        Self::enter_on(libc::STDIN_FILENO)
    }

    pub fn enter_on(fd: RawFd) -> io::Result<Self> {
        // Not a tty (pipe, redirect, CI): inert guard. isatty returns 0 and
        // sets errno to ENOTTY in that case; we treat that specifically.
        // SAFETY: isatty takes any fd and only inspects it; errno checked below.
        if unsafe { libc::isatty(fd) } == 0 {
            let err = io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::ENOTTY) {
                return Ok(Self {
                    fd,
                    // SAFETY: termios is plain old data; an all-zero value is valid and never read for an inert guard.
                    original: unsafe { std::mem::zeroed() },
                    active: false,
                });
            }
            return Err(err);
        }

        // SAFETY: termios is plain old data; zeroed is a valid initial value.
        let mut original: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: fd is a tty (checked above) and original points to a valid termios; result checked.
        if unsafe { libc::tcgetattr(fd, &raw mut original) } != 0 {
            return Err(io::Error::last_os_error());
        }

        let mut raw = original;
        // Local flags: no canonical mode, no echo, no signal generation
        // (^C, ^Z, ^\), no extended input processing (^V, ^O).
        raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG | libc::IEXTEN);
        // Input flags: no flow control, no CR->NL translation, no BREAK
        // handling, no parity check, no high-bit stripping.
        raw.c_iflag &= !(libc::IXON | libc::ICRNL | libc::BRKINT | libc::INPCK | libc::ISTRIP);
        // Output flags: raw output. Guest-produced \n stays \n, not \r\n.
        raw.c_oflag &= !libc::OPOST;
        // VMIN=1, VTIME=0: read(2) blocks until one byte arrives.
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;

        // SAFETY: fd is a tty and raw is a fully initialized termios; result checked.
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw const raw) } != 0 {
            return Err(io::Error::last_os_error());
        }

        install_signal_handlers(fd, &original);

        Ok(Self {
            fd,
            original,
            active: true,
        })
    }

    pub fn restore(&mut self) {
        if self.active {
            // TCSADRAIN waits until all queued output has been transmitted.
            // SAFETY: self.fd is the tty we configured and self.original is a valid saved termios.
            let mut ret =
                unsafe { libc::tcsetattr(self.fd, libc::TCSADRAIN, &raw const self.original) };
            if ret != 0 {
                // EINTR is harmless; we can retry once.
                let err = io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EINTR) {
                    // SAFETY: same valid fd and termios as above; retry after EINTR.
                    ret = unsafe {
                        libc::tcsetattr(self.fd, libc::TCSADRAIN, &raw const self.original)
                    };
                }
                if ret != 0 {
                    eprintln!("Warning: terminal restore failed: {err}");
                }
            }

            // SAFETY: self.fd is the valid tty fd; return value intentionally ignored.
            unsafe {
                libc::tcflush(self.fd, libc::TCIOFLUSH);
            }

            self.active = false;

            disarm_signal_handler();
        }
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        printcrln!("[terminal] Restoring terminal state");
        self.restore();
    }
}

static SIGNAL_INSTALL: Once = Once::new();
static SAVED_TERMIOS: AtomicPtr<libc::termios> = AtomicPtr::new(std::ptr::null_mut());
static SAVED_FD: AtomicI32 = AtomicI32::new(-1);
static HANDLER_DISARMED: AtomicBool = AtomicBool::new(false);

pub fn disarm_signal_handler() {
    HANDLER_DISARMED.store(true, Ordering::SeqCst);
}

fn install_signal_handlers(fd: RawFd, original: &libc::termios) {
    SIGNAL_INSTALL.call_once(|| {
        let termios_ptr = Box::into_raw(Box::new(*original));
        SAVED_TERMIOS.store(termios_ptr, Ordering::SeqCst);
        SAVED_FD.store(fd, Ordering::SeqCst);
        for &sig in FATAL_SIGNALS {
            install_one(sig);
        }
    });
}

/// Signals for which we want to restore before dying.
const FATAL_SIGNALS: &[libc::c_int] = &[
    libc::SIGINT, // ^C — though with ISIG off the guest sees it; still handle for other senders
    libc::SIGTERM,
    libc::SIGQUIT,
    libc::SIGHUP,
    libc::SIGPIPE, // e.g. stdout closed
    libc::SIGSEGV,
    libc::SIGBUS,
    libc::SIGABRT,
];

fn install_one(sig: libc::c_int) {
    // SAFETY: sigaction is plain old data; zeroed is a valid initial value.
    let mut sa: libc::sigaction = unsafe { std::mem::zeroed() };
    sa.sa_sigaction = handler as *const () as usize;
    // SA_RESETHAND: after we handle it, restore SIG_DFL so re-raise actually
    // terminates. SA_NODEFER: don't block the signal during handling, so a
    // nested same-signal re-raise goes through to default.
    sa.sa_flags = libc::SA_RESETHAND | libc::SA_NODEFER;
    // SAFETY: sa.sa_mask is a valid sigset_t to initialize.
    unsafe { libc::sigemptyset(&raw mut sa.sa_mask) };
    // SAFETY: sig is a valid signal number and sa is fully initialized; null old-action is allowed.
    unsafe { libc::sigaction(sig, &raw const sa, std::ptr::null_mut()) };
}

extern "C" fn handler(sig: libc::c_int) {
    if !HANDLER_DISARMED.load(Ordering::SeqCst) {
        let fd = SAVED_FD.load(Ordering::SeqCst);
        let t_ptr = SAVED_TERMIOS.load(Ordering::SeqCst);
        if fd >= 0 && !t_ptr.is_null() {
            // tcsetattr is on the POSIX list of async-signal-safe functions.
            // SAFETY: fd is the saved tty (>= 0, checked) and t_ptr points to the leaked termios; tcsetattr is async-signal-safe.
            unsafe { libc::tcsetattr(fd, libc::TCSANOW, t_ptr) };
        }
    }
    // SA_RESETHAND installed SIG_DFL already; re-raise so the process dies
    // with the correct status.
    // SAFETY: sig is the valid signal we are handling; raise re-delivers it to this process.
    unsafe { libc::raise(sig) };
}

const CTRL_A: u8 = 0x01;

#[derive(Debug, PartialEq, Eq)]
pub enum EscapeAction {
    /// Forward this byte to the guest (pass it to the serial RX FIFO).
    Forward(u8),
    /// User requested VMM exit (Ctrl-A x).
    Quit,
    /// User requested help (Ctrl-A ?). Caller decides what to print.
    Help,
    /// The byte was part of an escape sequence; no guest-visible effect.
    Swallowed,
}

pub struct EscapeState {
    prefix_seen: bool,
}

impl EscapeState {
    pub const fn new() -> Self {
        Self { prefix_seen: false }
    }

    pub const fn feed(&mut self, byte: u8) -> EscapeAction {
        if self.prefix_seen {
            self.prefix_seen = false;
            match byte {
                b'x' | b'X' => EscapeAction::Quit,
                b'?' => EscapeAction::Help,
                other => EscapeAction::Forward(other),
            }
        } else if byte == CTRL_A {
            self.prefix_seen = true;
            EscapeAction::Swallowed
        } else {
            EscapeAction::Forward(byte)
        }
    }
}

impl Default for EscapeState {
    fn default() -> Self {
        Self::new()
    }
}
