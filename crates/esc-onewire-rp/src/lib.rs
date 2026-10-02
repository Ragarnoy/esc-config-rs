//! Half-duplex one-wire UART on an RP2040/RP235x PIO block, for AM32 bootloaders,
//! implementing [`esc_session::Link`].
//!
//! Two state machines share one pin: TX drives 8N1 frames, RX samples them. The
//! line idles high: driven by TX while sending, held by the pad pull-up (and the
//! ESC's own pull-up) while receiving. A line held low for 20 ms makes an AM32
//! bootloader start its app, so never leave the pin low or floating.
//!
//! The PIO clock divider comes from `clk_sys_freq()`, so the same code runs at the
//! relay's 150 MHz and Elle's 200 MHz.
#![no_std]

use core::convert::Infallible;

use embassy_futures::select::{Either, select};
use embassy_rp::Peri;
use embassy_rp::gpio::{Level, Pull};
use embassy_rp::pio::program::pio_asm;
use embassy_rp::pio::{
    Common, Config, Direction, Instance, Pin, PioPin, ShiftConfig, ShiftDirection, StateMachine,
};
use embassy_time::{Duration, Instant, Timer};
use esc_session::Link;
use fixed::FixedU32;
use fixed::types::extra::U8;

/// AM32 bootloader baud rate.
pub const BAUD: u32 = 19_200;

/// PIO cycles per bit in both programs.
const CYCLES_PER_BIT: u32 = 14;

/// PIO clock divider for `baud` at system clock `clk_hz`, as 16.8 fixed-point bits.
///
/// `clk / (baud × 14)` with 8 fractional bits, rounded to nearest.
pub const fn divider_bits(clk_hz: u32, baud: u32) -> u32 {
    let num = clk_hz as u64 * 256;
    let den = baud as u64 * CYCLES_PER_BIT as u64;
    ((num + den / 2) / den) as u32
}

/// Effective baud for a divider: `clk × 256 / (bits × 14)`.
pub const fn effective_baud(clk_hz: u32, bits: u32) -> u32 {
    ((clk_hz as u64 * 256) / (bits as u64 * CYCLES_PER_BIT as u64)) as u32
}

// The divider must land within 0.1% of 19200 at every clock we run at (the old
// hardcoded 150 MHz divider gave 25.6 kbaud at 200 MHz).
const _: () = {
    let clocks = [125_000_000u32, 150_000_000, 200_000_000];
    let mut i = 0;
    while i < clocks.len() {
        let baud = effective_baud(clocks[i], divider_bits(clocks[i], BAUD));
        assert!(baud.abs_diff(BAUD) * 1000 < BAUD);
        i += 1;
    }
};

/// One-wire UART on PIO `P`, TX on state machine `TX`, RX on `RX`.
pub struct OneWireUart<'d, P: Instance, const TX: usize, const RX: usize> {
    sm_tx: StateMachine<'d, P, TX>,
    sm_rx: StateMachine<'d, P, RX>,
    pin: Pin<'d, P>,
}

impl<'d, P: Instance, const TX: usize, const RX: usize> OneWireUart<'d, P, TX, RX> {
    /// Set up the UART on `pin` at [`BAUD`], line driven high.
    pub fn new(
        pio: &mut Common<'d, P>,
        sm_tx: StateMachine<'d, P, TX>,
        sm_rx: StateMachine<'d, P, RX>,
        pin: Peri<'d, impl PioPin>,
    ) -> Self {
        let pin = pio.make_pio_pin(pin);
        Self::new_with_pin(pio, sm_tx, sm_rx, pin)
    }

    /// Set up the UART on an existing PIO pin at [`BAUD`], line driven high.
    pub fn new_with_pin(
        pio: &mut Common<'d, P>,
        mut sm_tx: StateMachine<'d, P, TX>,
        mut sm_rx: StateMachine<'d, P, RX>,
        mut pin: Pin<'d, P>,
    ) -> Self {
        pin.set_pull(Pull::Up);

        // TX: start bit, 8 data bits LSB first, stop bit; 14 cycles per bit.
        // `.side_set 1 opt` limits delays to 7, hence the split nops.
        let tx_prg = pio_asm!(
            ".side_set 1 opt",
            "pull block      side 1",     // wait for data, line high
            "set x, 7        side 0 [6]", // start bit
            "nop                   [6]",
            "bitloop:",
            "out pins, 1          [6]",
            "jmp x--, bitloop     [6]",
            "nop             side 1 [6]", // stop bit
            "nop                   [6]",
        );

        // RX: wait for the start bit, sample 8 bits mid-bit, wait for the line high.
        let rx_prg = pio_asm!(
            "wait 0 pin 0",
            "set x, 7        [20]", // 1.5 bit times to the middle of bit 0
            "bitloop:",
            "in pins, 1      [6]",
            "jmp x--, bitloop [6]",
            "push block",
            "wait 1 pin 0",
        );

        let clk = embassy_rp::clocks::clk_sys_freq();
        let divider = FixedU32::<U8>::from_bits(divider_bits(clk, BAUD));

        let mut tx_cfg = Config::default();
        tx_cfg.use_program(&pio.load_program(&tx_prg.program), &[&pin]);
        tx_cfg.set_out_pins(&[&pin]);
        tx_cfg.set_set_pins(&[&pin]);
        tx_cfg.clock_divider = divider;
        tx_cfg.shift_out = ShiftConfig {
            auto_fill: false,
            direction: ShiftDirection::Right,
            threshold: 32,
        };
        sm_tx.set_config(&tx_cfg);

        let mut rx_cfg = Config::default();
        rx_cfg.use_program(&pio.load_program(&rx_prg.program), &[]);
        rx_cfg.set_in_pins(&[&pin]);
        rx_cfg.clock_divider = divider;
        rx_cfg.shift_in = ShiftConfig {
            auto_fill: false,
            // Bits enter at the MSB; after 8 the byte is in bits 31..24.
            direction: ShiftDirection::Right,
            threshold: 8,
        };
        sm_rx.set_config(&rx_cfg);

        // Output high before the pin becomes an output, so it never glitches low.
        sm_tx.set_pins(Level::High, &[&pin]);
        sm_tx.set_pin_dirs(Direction::Out, &[&pin]);
        sm_tx.set_enable(true);

        Self { sm_tx, sm_rx, pin }
    }

    /// Send `data` as one burst, then release the line and enable RX.
    pub async fn write(&mut self, data: &[u8]) {
        self.sm_rx.set_enable(false);
        while self.sm_rx.rx().try_pull().is_some() {}

        self.sm_tx.set_pins(Level::High, &[&self.pin]);
        self.sm_tx.set_pin_dirs(Direction::Out, &[&self.pin]);
        if !self.sm_tx.is_enabled() {
            self.sm_tx.restart();
            self.sm_tx.set_enable(true);
        }

        // Idle high briefly before the first start bit.
        Timer::after_micros(100).await;

        for &byte in data {
            self.sm_tx.tx().wait_push(u32::from(byte)).await;
        }
        while !self.sm_tx.tx().empty() {
            Timer::after_micros(50).await;
        }
        // The last byte is still shifting out (10 bits = 521 µs at 19200).
        Timer::after_micros(530).await;

        // Turnaround: TX releases the pin, RX listens. The pull-ups hold it high.
        self.sm_tx.set_enable(false);
        self.sm_tx.set_pin_dirs(Direction::In, &[&self.pin]);
        self.sm_rx.set_pin_dirs(Direction::In, &[&self.pin]);
        while self.sm_rx.rx().try_pull().is_some() {}
        self.sm_rx.restart();
        self.sm_rx.set_enable(true);
    }

    /// Read up to `buf.len()` bytes until `deadline`; returns the count.
    pub async fn read_until(&mut self, buf: &mut [u8], deadline: Instant) -> usize {
        let mut n = 0;
        while n < buf.len() {
            match select(self.sm_rx.rx().wait_pull(), Timer::at(deadline)).await {
                Either::First(word) => {
                    buf[n] = (word >> 24) as u8;
                    n += 1;
                }
                Either::Second(()) => break,
            }
        }
        n
    }
}

impl<P: Instance, const TX: usize, const RX: usize> Link for OneWireUart<'_, P, TX, RX> {
    type Error = Infallible;

    async fn send(&mut self, frame: &[u8]) -> Result<(), Infallible> {
        self.write(frame).await;
        Ok(())
    }

    async fn recv(
        &mut self,
        buf: &mut [u8],
        timeout: esc_session::Duration,
    ) -> Result<usize, Infallible> {
        let deadline = Instant::now() + Duration::from_micros(u64::from(timeout.as_micros()));
        Ok(self.read_until(buf, deadline).await)
    }

    async fn delay(&mut self, duration: esc_session::Duration) {
        Timer::after_micros(u64::from(duration.as_micros())).await;
    }
}
