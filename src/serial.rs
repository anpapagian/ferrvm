//! 16550A UART device model.
//!
//! Implements the full programming-visible register set: RBR/THR, IER, IIR,
//! FCR, LCR, MCR, LSR, MSR, SCR, plus DLAB-banked DLL/DLM.
use std::collections::VecDeque;
use std::io::Write;
use std::sync::{Arc, Mutex};

use crate::bus::BusDevice;
use crate::traits::{self, Vm};

// ---- Register offsets (DLAB=0 unless noted) --------------------------------
const REG_RBR_THR_DLL: u64 = 0; // read: RBR, write: THR; DLAB=1: DLL
const REG_IER_DLM: u64 = 1; //      IER;            DLAB=1: DLM
const REG_IIR_FCR: u64 = 2; // read: IIR, write: FCR
const REG_LCR: u64 = 3;
const REG_MCR: u64 = 4;
const REG_LSR: u64 = 5;
const REG_MSR: u64 = 6;
const REG_SCR: u64 = 7;

// ---- IER bits --------------------------------------------------------------
const IER_ERBFI: u8 = 1 << 0; // Received data available
const IER_ETBEI: u8 = 1 << 1; // Transmitter holding register empty
const IER_ELSI: u8 = 1 << 2; // Receiver line status
const IER_EDSSI: u8 = 1 << 3; // Modem status
const IER_MASK: u8 = 0x0f;

// ---- IIR identification codes (bits 3:0 when bit 0 = 0 means "pending") ----
// Bit 0 is INVERTED: 0 = interrupt pending, 1 = no interrupt pending.
const IIR_NONE: u8 = 0x01;
const IIR_RLS: u8 = 0x06; // receiver line status (highest priority)
const IIR_RDA: u8 = 0x04; // received data available
const IIR_CTI: u8 = 0x0c; // character timeout (RX FIFO, no trigger reached)
const IIR_THRE: u8 = 0x02; // transmitter empty
const IIR_MS: u8 = 0x00; // modem status (lowest)

// Bits 6-7 read as 11 when FIFOs are enabled (FCR bit 0 set), identifying
// the part as a 16550A.
const IIR_FIFO_ENABLED: u8 = 0xc0;

// ---- FCR bits (write-only) -------------------------------------------------
const FCR_ENABLE: u8 = 1 << 0;
const FCR_CLEAR_RX: u8 = 1 << 1;
const FCR_CLEAR_TX: u8 = 1 << 2;
// bit 3: DMA mode select (ignored)
// bits 6-7: RX trigger level (1/4/8/14 bytes) — stored but not strictly
// enforced; we assert RDA on any byte. Close enough for the Linux driver.

// ---- LCR bits --------------------------------------------------------------
const LCR_DLAB: u8 = 1 << 7;

// ---- MCR bits --------------------------------------------------------------
const MCR_DTR: u8 = 1 << 0;
const MCR_RTS: u8 = 1 << 1;
const MCR_OUT1: u8 = 1 << 2;
const MCR_OUT2: u8 = 1 << 3; // gates IRQ line on real HW; we honor it.
const MCR_LOOPBACK: u8 = 1 << 4;
const MCR_MASK: u8 = 0x1f;

// ---- LSR bits --------------------------------------------------------------
const LSR_DR: u8 = 1 << 0; // data ready
const LSR_OE: u8 = 1 << 1; // overrun error
const LSR_THRE: u8 = 1 << 5; // transmitter holding register empty
const LSR_TEMT: u8 = 1 << 6; // transmitter empty (holding + shift)

// ---- MSR bits --------------------------------------------------------------
// Deltas (bits 0-3) and current states (bits 4-7).
const MSR_DCTS: u8 = 1 << 0;
const MSR_DDSR: u8 = 1 << 1;
const MSR_TERI: u8 = 1 << 2;
const MSR_DDCD: u8 = 1 << 3;
const MSR_CTS: u8 = 1 << 4;
const MSR_DSR: u8 = 1 << 5;
const MSR_RI: u8 = 1 << 6;
const MSR_DCD: u8 = 1 << 7;
const MSR_DELTAS: u8 = 0x0f;
// Sensible always-connected default so the driver doesn't think the line
// is dead: CTS|DSR|DCD asserted, RI deasserted.
const MSR_CONNECTED_DEFAULT: u8 = MSR_CTS | MSR_DSR | MSR_DCD;

// ---- FIFO size -------------------------------------------------------------
const FIFO_CAP: usize = 4096;

/// How long the RX FIFO must be idle (no injects, no RBR reads) with a
/// non-empty but below-trigger count before CTI fires. Real 16550A uses
/// "4 character times" at programmed baud; we can't emulate that
/// meaningfully against a user-typed stdin. 1ms feels instant to humans
/// and is longer than realistic inter-key jitter inside a paste burst.
pub const CTI_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1);

/// Translate the 2-bit FCR RX trigger code (bits 7:6) to a byte count.
/// Per NS16550A datasheet: 00 = 1, 01 = 4, 10 = 8, 11 = 14 bytes.
const fn trigger_bytes(code: u8) -> usize {
    match code & 0x03 {
        0 => 1,
        1 => 4,
        2 => 8,
        _ => 14,
    }
}

/// Abstracts the mechanism that delivers a line-level interrupt to the guest.
pub trait IrqSink: Send + Sync {
    fn set_level(&self, asserted: bool);
}

/// No-op sink used as default. Also useful in tests that don't care about
/// interrupt timing.
#[derive(Default)]
pub struct NullIrqSink;
impl IrqSink for NullIrqSink {
    fn set_level(&self, _asserted: bool) {}
}

#[allow(clippy::struct_excessive_bools)]
pub struct Serial {
    // Programmable registers
    ier: u8,
    lcr: u8,
    mcr: u8,
    scr: u8,
    dll: u8,
    dlm: u8,
    fcr_enabled: bool, // bit 0 of last FCR write
    rx_trigger: u8,    // bits 7:6 of last FCR write (0=1, 1=4, 2=8, 3=14 bytes)

    // Latched / computed status
    msr: u8,

    // FIFO (RX); TX is synchronous so no FIFO needed.
    rx: VecDeque<u8>,

    // Overrun latched into LSR until LSR is read.
    overrun: bool,

    // Latched "THR has been emptied" event. THR-empty interrupts are
    // edge-like: a one-shot fires after a THR write once transmission
    // completes. We fire on the write itself (instant send) and keep the
    // pending bit set until THR-interrupt is acknowledged (IIR read or THR
    // write).
    thre_pending: bool,

    // Character Timeout Indication. Set by `tick_cti` when rx has data but
    // the count is below trigger and no RX activity has occurred for
    // CTI_TIMEOUT. Cleared when the FIFO drains to empty.
    cti_pending: bool,

    // Timestamp of the last RX FIFO activity (inject or RBR read). Used by
    // `tick_cti` to decide whether "4 character times" (we approximate as
    // CTI_TIMEOUT) have elapsed since the last event.
    last_rx_activity: std::time::Instant,

    // Tracks last-asserted IRQ level so we only call the sink on transitions.
    last_irq_level: bool,

    // Outbound byte sink (stdout when wired up in main). Replace at
    // construction time with a Vec<u8>-backed sink for tests.
    out: Box<dyn Write + Send>,
    irq: Arc<dyn IrqSink>,
}

impl Serial {
    pub fn new(out: Box<dyn Write + Send>, irq: Arc<dyn IrqSink>) -> Self {
        Self {
            ier: 0,
            lcr: 0,
            mcr: 0,
            scr: 0,
            dll: 0,
            dlm: 0,
            fcr_enabled: false,
            rx_trigger: 0,
            msr: MSR_CONNECTED_DEFAULT,
            rx: VecDeque::with_capacity(FIFO_CAP),
            overrun: false,
            thre_pending: false,
            cti_pending: false,
            last_rx_activity: std::time::Instant::now(),
            last_irq_level: false,
            out,
            irq,
        }
    }

    /// Host -> guest: push bytes into the RX FIFO. Dropped-with-overrun if
    /// full.
    pub fn inject_rx(&mut self, bytes: &[u8]) {
        // In loopback mode, the physical RX line is disconnected from the
        // outside world. Dropping inbound bytes matches real HW.
        if self.mcr & MCR_LOOPBACK != 0 {
            return;
        }
        for &b in bytes {
            if self.rx.len() >= FIFO_CAP {
                self.overrun = true;
                break;
            }
            self.rx.push_back(b);
        }
        // Any RX activity resets the CTI timer. If a CTI was pending, the
        // fresh byte either pushes us past trigger (RDA takes over) or
        // restarts the 4-character-time window.
        self.last_rx_activity = std::time::Instant::now();
        self.cti_pending = false;
        self.update_irq();
    }

    /// Called by the stdin reader (or any external driver) on a periodic
    /// timeout. If RX has residual bytes below the trigger and enough
    /// time has passed since the last activity, CTI fires.
    ///
    /// Returns true if this tick changed interrupt state — useful for
    /// callers that want to batch wake-ups.
    pub fn tick_cti(&mut self) -> bool {
        let was = self.cti_pending;
        if !self.rx.is_empty()
            && self.fcr_enabled
            && self.rx.len() < trigger_bytes(self.rx_trigger)
            && self.last_rx_activity.elapsed() >= CTI_TIMEOUT
        {
            self.cti_pending = true;
        }
        let changed = was != self.cti_pending;
        if changed {
            self.update_irq();
        }
        changed
    }

    /// Replace the IRQ sink. Used during setup to swap from `NullIrqSink`
    /// to an `EventFdIrqSink` once the irqfd is wired.
    pub fn replace_irq_sink(&mut self, sink: Arc<dyn IrqSink>) {
        // After a sink swap, our cached `last_irq_level` refers to the old
        // sink's state. Force a re-evaluation against the new sink.
        self.last_irq_level = false;
        self.irq = sink;
        self.update_irq();
    }

    const fn dlab(&self) -> bool {
        self.lcr & LCR_DLAB != 0
    }

    /// Compute LSR on read. Bits 1-4 (error bits) are latched until read,
    /// at which point they clear.
    fn read_lsr(&mut self) -> u8 {
        let mut lsr = LSR_THRE | LSR_TEMT;
        if !self.rx.is_empty() {
            lsr |= LSR_DR;
        }
        if self.overrun {
            lsr |= LSR_OE;
        }
        // Reading LSR clears the error/break bits (1..=4) per datasheet.
        self.overrun = false;
        // Reading LSR also clears the RLS interrupt pending condition; since
        // we only latch overrun, re-evaluate.
        self.update_irq();
        lsr
    }

    /// Decide the single highest-priority pending interrupt source.
    /// Returns None if no interrupt is pending. `Some(code)` is the raw IIR
    /// identification code (bits 3:0 of IIR, with bit 0 = 0 meaning pending).
    fn pending_interrupt(&self) -> Option<u8> {
        // Priority order per NS16550D datasheet:
        //   RLS > RDA/CTI > THRE > MS
        if self.ier & IER_ELSI != 0 && self.overrun {
            return Some(IIR_RLS);
        }
        // RDA and CTI share a priority slot and the same IER bit (ERBFI).
        // RDA takes precedence when the trigger is reached; CTI covers the
        // "residual bytes below trigger, line gone quiet" case.
        if self.ier & IER_ERBFI != 0 {
            let trig = if self.fcr_enabled {
                trigger_bytes(self.rx_trigger)
            } else {
                // 16450 mode (no FIFO): any byte raises RDA immediately.
                1
            };
            if self.rx.len() >= trig {
                return Some(IIR_RDA);
            }
            if self.cti_pending && !self.rx.is_empty() {
                return Some(IIR_CTI);
            }
        }
        if self.ier & IER_ETBEI != 0 && self.thre_pending {
            return Some(IIR_THRE);
        }
        if self.ier & IER_EDSSI != 0 && self.msr & MSR_DELTAS != 0 {
            return Some(IIR_MS);
        }
        None
    }

    /// Called after any state change that might alter interrupt status.
    /// Asserts/deasserts through the sink, but only on edges.
    fn update_irq(&mut self) {
        // MCR.OUT2 gates the interrupt line on real ISA UARTs. Linux sets
        // OUT2 before enabling interrupts; we honor it.
        let gated = self.mcr & MCR_OUT2 == 0;
        let asserted = !gated && self.pending_interrupt().is_some();
        if asserted != self.last_irq_level {
            self.last_irq_level = asserted;
            self.irq.set_level(asserted);
        }
    }

    /// Read IIR. Side effect: if the currently-indicated source is THR-empty,
    /// reading IIR clears it (edge-like), matching the datasheet:
    /// "THRE interrupt cleared by either read of IIR ... or write to THR".
    fn read_iir(&mut self) -> u8 {
        let id = self.pending_interrupt().unwrap_or(IIR_NONE);
        if id == IIR_THRE {
            self.thre_pending = false;
            // Interrupt source gone; update line.
            self.update_irq();
        }
        let fifo_bits = if self.fcr_enabled {
            IIR_FIFO_ENABLED
        } else {
            0
        };
        id | fifo_bits
    }

    /// THR write: either send to host out, or loop back into RX.
    fn write_thr(&mut self, byte: u8) {
        if self.mcr & MCR_LOOPBACK != 0 {
            // Internal loopback: byte appears at RBR. Overrun if FIFO full.
            if self.rx.len() >= FIFO_CAP {
                self.overrun = true;
            } else {
                self.rx.push_back(byte);
            }
        } else {
            // Synchronous emission. A write() error here is not the guest's
            // problem — we swallow it (stdout can be closed mid-run).
            let _ = self.out.write_all(&[byte]);
            let _ = self.out.flush();
        }
        // THR write also clears any pending THRE interrupt (write-to-THR is
        // one of the two acks), then we immediately re-latch it because the
        // holding register became empty again (sync transmit).
        self.thre_pending = true;
        self.update_irq();
    }

    fn write_fcr(&mut self, value: u8) {
        let enabled = value & FCR_ENABLE != 0;
        // Enable bit transitioning from 0 to 1 clears both FIFOs per spec.
        let transitioning_on = enabled && !self.fcr_enabled;

        self.fcr_enabled = enabled;
        self.rx_trigger = (value >> 6) & 0x03;

        if transitioning_on || value & FCR_CLEAR_RX != 0 {
            self.rx.clear();
            self.cti_pending = false;
        }
        // "Clear TX" has no effect because our TX is synchronous.
        let _ = value & FCR_CLEAR_TX;
        self.update_irq();
    }

    fn write_mcr(&mut self, value: u8) {
        let was_loopback = self.mcr & MCR_LOOPBACK != 0;
        self.mcr = value & MCR_MASK;
        let is_loopback = self.mcr & MCR_LOOPBACK != 0;

        // In loopback mode, MCR modem-control bits (DTR/RTS/OUT1/OUT2)
        // mirror into MSR current-state bits (DSR/CTS/RI/DCD), and changes
        // set the delta bits. This is what Linux's autoconfig exercises.
        if is_loopback {
            let new_msr_state = ((self.mcr & MCR_DTR)  << 5) |  // DTR  -> DSR  (bit 0 -> 5)
                                ((self.mcr & MCR_RTS)  << 3) |  // RTS  -> CTS  (bit 1 -> 4)
                                ((self.mcr & MCR_OUT1) << 4) |  // OUT1 -> RI   (bit 2 -> 6)
                                ((self.mcr & MCR_OUT2) << 4); // OUT2 -> DCD  (bit 3 -> 7)
            let old_state = self.msr & 0xf0;
            let deltas = old_state ^ new_msr_state;
            // Bits 4..7 delta -> bits 0..3 delta; RI delta is TERI (edge).
            let mut delta_bits = 0;
            if deltas & MSR_CTS != 0 {
                delta_bits |= MSR_DCTS;
            }
            if deltas & MSR_DSR != 0 {
                delta_bits |= MSR_DDSR;
            }
            if deltas & MSR_RI != 0 {
                delta_bits |= MSR_TERI;
            } // approx
            if deltas & MSR_DCD != 0 {
                delta_bits |= MSR_DDCD;
            }
            self.msr = new_msr_state | (self.msr & MSR_DELTAS) | delta_bits;

            // Entering loopback also drains any externally-queued RX to
            // match "physical line disconnected" semantics.
            if !was_loopback {
                self.rx.clear();
                self.cti_pending = false;
            }
        } else if was_loopback {
            // Leaving loopback: restore connected-line defaults, do not
            // re-assert deltas.
            self.msr = MSR_CONNECTED_DEFAULT | (self.msr & MSR_DELTAS);
        }

        self.update_irq();
    }

    fn read_msr(&mut self) -> u8 {
        let value = self.msr;
        // Reading MSR clears the delta bits.
        self.msr &= !MSR_DELTAS;
        self.update_irq();
        value
    }
}

impl BusDevice for Serial {
    fn read(&mut self, offset: u64, data: &mut [u8]) {
        // UART is a byte-wide device. Wider accesses (guest bug) read first
        // byte from the device, pad with 0xff.
        if data.is_empty() {
            return;
        }
        let byte = match offset {
            REG_RBR_THR_DLL if self.dlab() => self.dll,
            REG_RBR_THR_DLL => {
                let b = self.rx.pop_front().unwrap_or(0);
                // Any RBR read resets the inactivity timer. If the FIFO is
                // now empty, CTI cannot be pending.
                self.last_rx_activity = std::time::Instant::now();
                if self.rx.is_empty() {
                    self.cti_pending = false;
                }
                self.update_irq();
                b
            }
            REG_IER_DLM if self.dlab() => self.dlm,
            REG_IER_DLM => self.ier,
            REG_IIR_FCR => self.read_iir(),
            REG_LCR => self.lcr,
            REG_MCR => self.mcr,
            REG_LSR => self.read_lsr(),
            REG_MSR => self.read_msr(),
            REG_SCR => self.scr,
            _ => 0xff,
        };
        data[0] = byte;
        for b in data.iter_mut().skip(1) {
            *b = 0xff;
        }
    }

    fn write(&mut self, offset: u64, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let value = data[0];
        match offset {
            REG_RBR_THR_DLL if self.dlab() => self.dll = value,
            REG_RBR_THR_DLL => self.write_thr(value),
            REG_IER_DLM if self.dlab() => self.dlm = value,
            REG_IER_DLM => {
                self.ier = value & IER_MASK;
                self.update_irq();
            }
            REG_IIR_FCR => self.write_fcr(value),
            REG_LCR => self.lcr = value,
            REG_MCR => self.write_mcr(value),
            REG_SCR => self.scr = value,
            // LSR and MSR are read-only on real HW. Linux's autoconfig
            // writes to LSR/MSR at one point to probe for buggy clones;
            // we silently ignore.
            _ => {}
        }
    }
}

pub fn register_serial(vm: &dyn Vm, irq: u32, serial: &Arc<Mutex<Serial>>) -> traits::Result<()> {
    let sink = vm.register_irq(irq)?;
    serial
        .lock()
        .expect("serial poisoned")
        .replace_irq_sink(sink);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    struct TestIrq {
        level: AtomicBool,
    }

    impl TestIrq {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                level: AtomicBool::new(false),
            })
        }
        fn get(&self) -> bool {
            self.level.load(Ordering::SeqCst)
        }
    }

    impl IrqSink for TestIrq {
        fn set_level(&self, asserted: bool) {
            self.level.store(asserted, Ordering::SeqCst);
        }
    }

    fn setup_serial() -> (Serial, Arc<TestIrq>, Arc<Mutex<Vec<u8>>>) {
        struct OutWrapper(Arc<Mutex<Vec<u8>>>);
        impl Write for OutWrapper {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let out = Arc::new(Mutex::new(Vec::new()));
        let irq = TestIrq::new();
        let serial = Serial::new(Box::new(OutWrapper(out.clone())), irq.clone());
        (serial, irq, out)
    }

    #[test]
    fn test_serial_dlab() {
        let (mut serial, _, _) = setup_serial();

        // Write DLL while DLAB=1
        serial.write(REG_LCR, &[LCR_DLAB]);
        serial.write(REG_RBR_THR_DLL, &[0x34]);
        serial.write(REG_LCR, &[0]);

        // Check THR (DLAB=0) is NOT 0x34
        let mut data = [0u8; 1];
        serial.read(REG_RBR_THR_DLL, &mut data);
        assert_ne!(data[0], 0x34);

        // Read DLL back
        serial.write(REG_LCR, &[LCR_DLAB]);
        serial.read(REG_RBR_THR_DLL, &mut data);
        assert_eq!(data[0], 0x34);
    }

    #[test]
    fn test_serial_loopback() {
        let (mut serial, _, _) = setup_serial();

        // Enable loopback
        serial.write(REG_MCR, &[MCR_LOOPBACK]);

        // Write to THR, should appear in RBR
        serial.write(REG_RBR_THR_DLL, b"A");
        let mut data = [0u8; 1];
        serial.read(REG_RBR_THR_DLL, &mut data);
        assert_eq!(data[0], b'A');

        // Check modem loopback (DTR -> DSR)
        serial.write(REG_MCR, &[MCR_LOOPBACK | MCR_DTR]);
        serial.read(REG_MSR, &mut data);
        assert!(data[0] & MSR_DSR != 0);
        assert!(data[0] & MSR_DDSR != 0); // Delta bit should be set
    }

    #[test]
    fn test_serial_interrupts() {
        let (mut serial, irq, _) = setup_serial();

        // Enable RDA interrupt and MCR.OUT2
        serial.write(REG_IER_DLM, &[IER_ERBFI]);
        serial.write(REG_MCR, &[MCR_OUT2]);
        assert!(!irq.get());

        // Inject data, should trigger IRQ
        serial.inject_rx(b"X");
        assert!(irq.get());

        // Read IIR, should show RDA
        let mut data = [0u8; 1];
        serial.read(REG_IIR_FCR, &mut data);
        assert_eq!(data[0] & 0x0f, IIR_RDA);

        // Read RBR, IRQ should clear
        serial.read(REG_RBR_THR_DLL, &mut data);
        assert!(!irq.get());
    }

    #[test]
    fn test_serial_fifo_overrun() {
        let (mut serial, _, _) = setup_serial();
        serial.write(REG_IIR_FCR, &[FCR_ENABLE]);

        let mut data = [0u8; 1];
        for i in 0..FIFO_CAP {
            serial.inject_rx(&[i as u8]);
        }
        // LSR should NOT have OE yet
        serial.read(REG_LSR, &mut data);
        assert!(data[0] & LSR_OE == 0);

        // One more byte triggers overrun
        serial.inject_rx(b"!");
        serial.read(REG_LSR, &mut data);
        assert!(data[0] & LSR_OE != 0);
    }
}
