//! Stdin reader thread
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use crate::serial::Serial;
use crate::terminal::{EscapeAction, EscapeState};

#[derive(Clone, Default)]
pub struct Shutdown {
    inner: Arc<ShutdownInner>,
}

#[derive(Default)]
struct ShutdownInner {
    flag: AtomicBool,
    kick_threads: Mutex<Vec<libc::pthread_t>>,
}

impl Shutdown {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register this vCPU thread to receive SIGUSR1 shutdown kicks.
    pub fn add_kick_target(&self) {
        // SAFETY: pthread_self has no preconditions and returns the caller's id.
        let thread = unsafe { libc::pthread_self() };
        self.inner
            .kick_threads
            .lock()
            .expect("kick targets poisoned")
            .push(thread);
    }

    /// Mark shutdown requested and kick every registered vCPU thread.
    pub fn request(&self) {
        self.inner.flag.store(true, Ordering::SeqCst);
        // Always send the kick, even on re-entry: if a vCPU somehow raced
        // past an earlier kick back into KVM_RUN, a second signal bumps it
        // out again.
        let targets = self
            .inner
            .kick_threads
            .lock()
            .expect("kick targets poisoned");
        for &thread in targets.iter() {
            // SAFETY: vCPU threads stay alive until shutdown is observed and
            // they exit their run loops; the ids remain valid until then.
            unsafe { libc::pthread_kill(thread, libc::SIGUSR1) };
        }
    }

    /// Has shutdown been requested yet?
    pub fn requested(&self) -> bool {
        self.inner.flag.load(Ordering::SeqCst)
    }
}

const DEFAULT_INPUT_FD: libc::c_int = libc::STDIN_FILENO;

const HELP_TEXT: &str = "\r\n\
    ferrvm console:\r\n\
    \x20 Ctrl-A x      quit ferrvm\r\n\
    \x20 Ctrl-A Ctrl-A send literal Ctrl-A to guest\r\n\
    \x20 Ctrl-A ?      this help\r\n\
    \r\n";

pub fn spawn_stdin_reader(serial: Arc<Mutex<Serial>>, shutdown: Shutdown) -> JoinHandle<()> {
    spawn_reader_on_fd(DEFAULT_INPUT_FD, serial, shutdown)
}

pub fn spawn_reader_on_fd(
    fd: libc::c_int,
    serial: Arc<Mutex<Serial>>,
    shutdown: Shutdown,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("ferrvm-stdin".into())
        .spawn(move || reader_loop(fd, &serial, &shutdown))
        .expect("failed to spawn stdin reader thread")
}

fn reader_loop(fd: libc::c_int, serial: &Arc<Mutex<Serial>>, shutdown: &Shutdown) {
    const POLL_TIMEOUT_MS: libc::c_int = 2;

    let mut escape = EscapeState::new();
    // Read in small batches to reduce syscall overhead when the user pastes text.
    let mut buf = [0u8; 64];

    loop {
        if shutdown.requested() {
            return;
        }

        // Wait for input OR a short timeout.
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: pfd points to one valid pollfd and we pass a matching count of 1.
        let prc = unsafe { libc::poll(&raw mut pfd, 1, POLL_TIMEOUT_MS) };
        if prc < 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            shutdown.request();
            return;
        }
        if prc == 0 {
            serial.lock().expect("serial poisoned").tick_cti();
            continue;
        }

        // SAFETY: buf is writable for buf.len() bytes and fd is owned by this reader.
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            shutdown.request();
            return;
        }
        if n == 0 {
            shutdown.request();
            return;
        }

        // Collect forwardable bytes so we acquire the serial lock at most
        // once per batch. `inject_rx` takes a slice; batching matters more
        // here than it would for the escape machine because the serial lock
        // may be contended with vcpu threads polling LSR.
        let mut forward: [u8; 64] = [0; 64];
        let mut forward_len = 0usize;
        let mut quit = false;
        let mut help = false;

        for &byte in &buf[..n as usize] {
            match escape.feed(byte) {
                EscapeAction::Forward(b) => {
                    forward[forward_len] = b;
                    forward_len += 1;
                }
                EscapeAction::Swallowed => {}
                EscapeAction::Help => help = true,
                EscapeAction::Quit => {
                    quit = true;
                    break;
                }
            }
        }

        if forward_len > 0 {
            let mut s = serial.lock().expect("serial poisoned");
            s.inject_rx(&forward[..forward_len]);
        }

        if help {
            // Write to stderr (fd 2); stdout belongs to the guest. A failed
            // write here is not worth acting on.
            let _ = std::io::stderr().write_all(HELP_TEXT.as_bytes());
        }

        if quit {
            shutdown.request();
            return;
        }
    }
}
