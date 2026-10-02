//! Single-wire half-duplex UART using PIO.
//!
//! Used for BLHeli/AM32 bootloader communication where TX and RX
//! happen on the same signal wire.

use embassy_rp::Peri;
use embassy_rp::gpio::{Level, Pull};
use embassy_rp::pio::program::pio_asm;
use embassy_rp::pio::{
    Common, Config, Direction, Instance, Pin, PioPin, ShiftConfig, ShiftDirection, StateMachine,
};
use fixed::traits::ToFixed;

/// Single-wire UART for ESC bootloader communication.
pub struct OneWireUart<'d, PIO: Instance, const SM_TX: usize, const SM_RX: usize> {
    sm_tx: StateMachine<'d, PIO, SM_TX>,
    sm_rx: StateMachine<'d, PIO, SM_RX>,
    pin: Pin<'d, PIO>,
}

impl<'d, PIO: Instance, const SM_TX: usize, const SM_RX: usize> OneWireUart<'d, PIO, SM_TX, SM_RX> {
    /// Create a new single-wire UART on the given pin (takes ownership of Peri).
    pub fn new(
        pio: &mut Common<'d, PIO>,
        sm_tx: StateMachine<'d, PIO, SM_TX>,
        sm_rx: StateMachine<'d, PIO, SM_RX>,
        pin: Peri<'d, impl PioPin>,
        baud: u32,
    ) -> Self {
        let pio_pin = pio.make_pio_pin(pin);
        Self::new_with_pin(pio, sm_tx, sm_rx, pio_pin, baud)
    }

    /// Create a new single-wire UART using an existing PIO pin.
    ///
    /// Uses two state machines: one for TX, one for RX.
    /// Default baud rate is 19200 for BLHeli/AM32 bootloader.
    pub fn new_with_pin(
        pio: &mut Common<'d, PIO>,
        mut sm_tx: StateMachine<'d, PIO, SM_TX>,
        mut sm_rx: StateMachine<'d, PIO, SM_RX>,
        mut pin: Pin<'d, PIO>,
        baud: u32,
    ) -> Self {
        // Enable pull-up on the pin for proper single-wire operation
        // The line idles high, pull-up ensures it stays high when released
        pin.set_pull(Pull::Up);

        // TX program: shift out 8 bits with start/stop bits
        // Format: start bit (0), 8 data bits LSB first, stop bit (1)
        // Each bit must be exactly 14 PIO cycles for correct timing
        // Note: .side_set 1 opt limits delay to max 7, so we split into nops
        let tx_prg = pio_asm!(
            ".side_set 1 opt",
            // Wait for data in FIFO, then send start bit
            "pull block      side 1", // Wait for data, keep line high (idle)
            "set x, 7        side 0 [6]", // Start bit low, 7 cycles
            "nop                   [6]", // 7 more cycles = 14 total for start bit
            "bitloop:",
            "out pins, 1          [6]",   // Output data bit (7 cycles)
            "jmp x--, bitloop     [6]",   // Loop for 8 bits (7 cycles) = 14 cycles/bit
            "nop             side 1 [6]", // Stop bit high, 7 cycles
            "nop                   [6]",  // 7 more cycles = 14 total for stop bit
        );

        // RX program: sample 8 bits after detecting start bit
        // With 14 cycles per bit, we need to wait 1.5 bit times (21 cycles)
        // to sample in the middle of the first data bit
        //
        // Using `pin 0` to read via IN_BASE pin mapping (matches the configured pin).
        let rx_prg = pio_asm!(
            // Wait for start bit (falling edge) on the configured input pin
            "wait 0 pin 0",
            "set x, 7        [20]", // Wait 1.5 bit times (21 cycles) to center on first data bit
            "bitloop:",
            "in pins, 1      [6]",  // Sample bit (7 cycles)
            "jmp x--, bitloop [6]", // Loop for 8 bits (7 cycles) = 14 cycles per bit
            "push block",           // Push received byte
            // Wait for stop bit / line to go high before next byte
            "wait 1 pin 0",
        );

        // Calculate clock divider for baud rate
        // PIO runs at system clock (150MHz for RP2350)
        // Each bit takes ~14 cycles in our program:
        // - out/in instruction with [6] delay = 7 cycles
        // - jmp instruction with [6] delay = 7 cycles
        // Total = 14 cycles per bit
        let sys_clock = 150_000_000u32;
        let cycles_per_bit = 14u32;
        let div = sys_clock / (baud * cycles_per_bit);

        // Configure TX state machine
        let mut tx_cfg = Config::default();
        tx_cfg.use_program(&pio.load_program(&tx_prg.program), &[&pin]);
        tx_cfg.set_out_pins(&[&pin]);
        tx_cfg.set_set_pins(&[&pin]);
        tx_cfg.clock_divider = (div as u16).to_fixed();
        tx_cfg.shift_out = ShiftConfig {
            auto_fill: false,
            direction: ShiftDirection::Right, // LSB first
            threshold: 32,
        };
        sm_tx.set_config(&tx_cfg);
        // Set output value HIGH first, THEN we can set direction to OUT without glitching LOW
        sm_tx.set_pins(Level::High, &[&pin]);

        // Configure RX state machine
        let mut rx_cfg = Config::default();
        rx_cfg.use_program(&pio.load_program(&rx_prg.program), &[]);
        rx_cfg.set_in_pins(&[&pin]);
        rx_cfg.clock_divider = (div as u16).to_fixed();
        rx_cfg.shift_in = ShiftConfig {
            auto_fill: false,
            // UART sends LSB first. ShiftRight means new bits enter at MSB,
            // and after 8 shifts the byte is in the correct order.
            direction: ShiftDirection::Right,
            threshold: 8,
        };
        sm_rx.set_config(&rx_cfg);
        // Don't set RX pin direction here - leave TX in control

        // Start TX to actively drive the line HIGH
        // TX program sits at `pull block side 1`, driving HIGH while waiting for data
        // This is more reliable than relying on pull-up alone
        sm_tx.set_pins(Level::High, &[&pin]);
        sm_tx.set_pin_dirs(Direction::Out, &[&pin]);
        sm_tx.set_enable(true);

        Self { sm_tx, sm_rx, pin }
    }

    /// Send bytes over the single wire.
    ///
    /// TX is already driving HIGH. Push data, wait for TX, then switch to RX.
    pub async fn write(&mut self, data: &[u8]) {
        // TX is already enabled and driving HIGH (sitting at `pull block side 1`)
        // Just disable RX if it was enabled from a previous read
        self.sm_rx.set_enable(false);

        // Clear any stale data in RX FIFO
        while self.sm_rx.rx().try_pull().is_some() {}

        // Ensure TX is in the right state (it should be sitting at `pull block side 1`)
        // If it's not, restart it
        self.sm_tx.set_pins(Level::High, &[&self.pin]);
        self.sm_tx.set_pin_dirs(Direction::Out, &[&self.pin]);
        if !self.sm_tx.is_enabled() {
            self.sm_tx.restart();
            self.sm_tx.set_enable(true);
        }

        // Small delay to ensure line is HIGH/idle before first byte
        embassy_time::Timer::after_micros(100).await;

        // Send each byte
        for &byte in data {
            self.sm_tx.tx().wait_push(byte as u32).await;
        }

        // Wait for TX FIFO to empty (bytes are in shift register now)
        while !self.sm_tx.tx().empty() {
            embassy_time::Timer::after_micros(50).await;
        }

        // Last byte is being transmitted by shift register (~521µs at 19200 baud)
        // ESC responds ~188µs after we finish, so we need to be ready quickly
        // Wait just enough for the last byte to complete
        embassy_time::Timer::after_micros(530).await;

        // CRITICAL: Turnaround sequence
        // TX just finished stop bit, line is HIGH
        // 1. Disable TX and release its output claim
        // 2. Set pin to input so RX can read the ESC response
        self.sm_tx.set_enable(false);
        self.sm_tx.set_pin_dirs(Direction::In, &[&self.pin]); // TX releases output
        self.sm_rx.set_pin_dirs(Direction::In, &[&self.pin]); // RX reads input

        // Clear any garbage and enable RX immediately
        while self.sm_rx.rx().try_pull().is_some() {}
        self.sm_rx.restart();
        self.sm_rx.set_enable(true);
    }

    /// Switch back to TX mode after receiving.
    ///
    /// This re-enables TX to actively drive the line HIGH (idle state).
    /// Call this after reading the response to prepare for the next command.
    pub fn switch_to_tx_idle(&mut self) {
        self.sm_rx.set_enable(false);
        // Set output HIGH first, then direction to OUT, then enable TX
        self.sm_tx.set_pins(Level::High, &[&self.pin]);
        self.sm_tx.set_pin_dirs(Direction::Out, &[&self.pin]);
        self.sm_tx.restart();
        self.sm_tx.set_enable(true);
    }

    /// Try to read a byte if available (non-blocking).
    pub fn try_read(&mut self) -> Option<u8> {
        // With ShiftRight, data is at bits 31-24, need to shift down
        self.sm_rx.rx().try_pull().map(|v| (v >> 24) as u8)
    }

    /// Read a byte, waiting until one is available.
    pub async fn read(&mut self) -> u8 {
        // With ShiftRight, data is at bits 31-24, need to shift down
        (self.sm_rx.rx().wait_pull().await >> 24) as u8
    }

    /// Read bytes with timeout.
    ///
    /// Uses `timeout_ms` to wait for the first byte, then a short inter-byte
    /// timeout (3ms) for subsequent bytes. At 19200 baud one byte takes ~521µs,
    /// so a 3ms gap reliably indicates end of response.
    ///
    /// Returns the number of bytes read.
    pub async fn read_with_timeout(&mut self, buf: &mut [u8], timeout_ms: u64) -> usize {
        use embassy_futures::select::{Either, select};
        use embassy_time::{Duration, Timer};

        /// Inter-byte timeout: 3ms is ~6x the byte time at 19200 baud.
        const INTER_BYTE_TIMEOUT_MS: u64 = 3;

        let mut count = 0;
        let initial_deadline = embassy_time::Instant::now() + Duration::from_millis(timeout_ms);

        while count < buf.len() {
            let timeout = if count == 0 {
                // First byte: use the full initial timeout
                let remaining =
                    initial_deadline.saturating_duration_since(embassy_time::Instant::now());
                if remaining.as_ticks() == 0 {
                    break;
                }
                remaining
            } else {
                // Subsequent bytes: short inter-byte timeout
                Duration::from_millis(INTER_BYTE_TIMEOUT_MS)
            };

            match select(self.sm_rx.rx().wait_pull(), Timer::after(timeout)).await {
                Either::First(byte) => {
                    buf[count] = (byte >> 24) as u8;
                    count += 1;
                }
                Either::Second(_) => break, // Timeout
            }
        }

        count
    }

    /// Read exactly n bytes with timeout.
    ///
    /// Returns the number of bytes read (may be less than n if timeout).
    pub async fn read_exact_with_timeout(
        &mut self,
        buf: &mut [u8],
        n: usize,
        timeout_ms: u64,
    ) -> usize {
        use embassy_futures::select::{Either, select};
        use embassy_time::{Duration, Timer};

        let mut count = 0;
        let to_read = n.min(buf.len());
        let deadline = embassy_time::Instant::now() + Duration::from_millis(timeout_ms);

        while count < to_read {
            let remaining = deadline.saturating_duration_since(embassy_time::Instant::now());
            if remaining.as_ticks() == 0 {
                break;
            }

            match select(self.sm_rx.rx().wait_pull(), Timer::after(remaining)).await {
                Either::First(byte) => {
                    buf[count] = (byte >> 24) as u8;
                    count += 1;
                }
                Either::Second(_) => break, // Timeout
            }
        }

        count
    }
}
