//! DShot output using RP2350 PIO.
//!
//! Uses a PIO state machine to generate precise DShot timing.
//! The PIO program shifts out 16 bits with proper pulse widths.

use embassy_rp::Peri;
use embassy_rp::pio::program::pio_asm;
use embassy_rp::pio::{
    Common, Config, Direction, FifoJoin, Instance, PioPin, ShiftConfig, ShiftDirection,
    StateMachine,
};
use esc_dshot::Speed;

use embassy_rp::pio::Pin;

/// DShot output using PIO.
pub struct DshotPio<'d, PIO: Instance, const SM: usize> {
    sm: StateMachine<'d, PIO, SM>,
    pin: Pin<'d, PIO>,
}

impl<'d, PIO: Instance, const SM: usize> DshotPio<'d, PIO, SM> {
    /// Create a new DShot output on the given pin (takes ownership of Peri).
    pub fn new(
        pio: &mut Common<'d, PIO>,
        sm: StateMachine<'d, PIO, SM>,
        pin: Peri<'d, impl PioPin>,
        speed: Speed,
    ) -> Self {
        let pio_pin = pio.make_pio_pin(pin);
        Self::new_with_pin(pio, sm, pio_pin, speed)
    }

    /// Create a new DShot output using an existing PIO pin.
    ///
    /// This allows reusing a pin that was already converted to a PIO pin.
    pub fn new_with_pin(
        pio: &mut Common<'d, PIO>,
        mut sm: StateMachine<'d, PIO, SM>,
        out_pin: Pin<'d, PIO>,
        speed: Speed,
    ) -> Self {
        // DShot bit encoding:
        // - "0" bit: high for ~35% of bit period, low for rest
        // - "1" bit: high for ~75% of bit period, low for rest

        // PIO program for DShot
        // Each bit: pull value, set high, delay based on bit value, set low, delay rest
        //
        // Simplified approach: use autopull and encode timing in the values
        // We'll precompute the frame with timing embedded

        // For now, use a simpler bitbanging approach via PIO
        // The program will:
        // 1. Pull 32 bits from FIFO (we send frame in upper 16 bits)
        // 2. For each of 16 bits: output high, delay, output low, delay

        let prg = pio_asm!(
            ".side_set 1",
            ".wrap_target",
            // Pull frame (blocking), shift out 16 bits
            "pull block      side 0",
            // Loop for 16 bits
            "set x, 15       side 0",
            "bitloop:",
            // Shift out MSB first, branch on bit value
            "out y, 1        side 0",
            "jmp !y, bit0    side 1", // Start high pulse
            // Bit 1: longer high pulse (~74% high)
            "bit1:",
            "nop             side 1 [12]",
            "jmp endbit      side 0 [2]",
            // Bit 0: shorter high pulse (~37% high)
            "bit0:",
            "nop             side 1 [5]",
            "nop             side 0 [9]",
            "endbit:",
            "jmp x--, bitloop side 0",
            ".wrap",
        );

        let mut cfg = Config::default();
        cfg.use_program(&pio.load_program(&prg.program), &[&out_pin]);

        // Configure clock divider for bit timing using fractional precision
        cfg.clock_divider = Self::calc_divider(speed);

        cfg.shift_out = ShiftConfig {
            auto_fill: false,
            direction: ShiftDirection::Left,
            threshold: 32,
        };
        cfg.fifo_join = FifoJoin::TxOnly;

        sm.set_config(&cfg);
        sm.set_pin_dirs(Direction::Out, &[&out_pin]);
        sm.set_enable(true);

        Self { sm, pin: out_pin }
    }

    /// Change DShot speed by adjusting the clock divider.
    ///
    /// This does not reload the PIO program, just changes timing.
    /// Uses fractional divider for better accuracy.
    pub fn set_speed(&mut self, speed: Speed) {
        self.sm.set_enable(false);
        self.sm.restart();
        self.sm.set_clock_divider(Self::calc_divider(speed));
        self.sm.set_enable(true);
    }

    /// Calculate the clock divider for a given DShot speed.
    /// Uses fractional divider (8-bit fraction) for better accuracy.
    fn calc_divider(speed: Speed) -> fixed::FixedU32<fixed::types::extra::U8> {
        // Target bit cycles at 150MHz system clock
        let bit_cycles: u32 = match speed {
            Speed::DShot150 => 1000,
            Speed::DShot300 => 500,
            Speed::DShot600 => 250,
            Speed::DShot1200 => 125,
        };

        // PIO cycles per bit (from program analysis)
        const CYCLES_PER_BIT_PIO: u32 = 19;

        // Calculate divider with 8-bit fractional precision
        // divider = bit_cycles / CYCLES_PER_BIT_PIO
        // To get 8-bit fraction: (bit_cycles * 256) / CYCLES_PER_BIT_PIO
        let div_fixed_raw = (bit_cycles << 8) / CYCLES_PER_BIT_PIO;

        // Convert to FixedU32<U8> - the raw value already has 8 fractional bits
        fixed::FixedU32::<fixed::types::extra::U8>::from_bits(div_fixed_raw)
    }

    /// Release resources for reuse.
    ///
    /// Returns the state machine and pin so they can be used for other purposes.
    pub fn release(mut self) -> (StateMachine<'d, PIO, SM>, Pin<'d, PIO>) {
        self.sm.set_enable(false);
        (self.sm, self.pin)
    }

    /// Send a DShot frame.
    ///
    /// The frame should be a complete 16-bit DShot frame (from `esc_dshot::encode`).
    pub async fn send_frame(&mut self, frame: u16) {
        // Pack frame into upper 16 bits (PIO shifts out MSB first)
        let data = (frame as u32) << 16;
        self.sm.tx().wait_push(data).await;
    }
}

/// Simpler DShot implementation for initial testing.
///
/// This version uses basic GPIO toggling with delays.
/// Less precise but easier to debug.
pub mod bitbang {
    use embassy_rp::gpio::{Level, Output, Pin};
    use embassy_rp::Peri;
    use embassy_time::{Duration, Timer};
    use esc_dshot::Speed;

    pub struct DshotBitbang<'d> {
        pin: Output<'d>,
        t0_high_ns: u64,
        t0_low_ns: u64,
        t1_high_ns: u64,
        t1_low_ns: u64,
    }

    impl<'d> DshotBitbang<'d> {
        pub fn new(pin: Peri<'d, impl Pin>, speed: Speed) -> Self {
            let bit_period = speed.bit_period_ns() as u64;
            let t0_high_ns = bit_period * 35 / 100;
            let t0_low_ns = bit_period - t0_high_ns;
            let t1_high_ns = bit_period * 75 / 100;
            let t1_low_ns = bit_period - t1_high_ns;

            Self {
                pin: Output::new(pin, Level::Low),
                t0_high_ns,
                t0_low_ns,
                t1_high_ns,
                t1_low_ns,
            }
        }

        pub async fn send_frame(&mut self, frame: u16) {
            // Send MSB first
            for i in (0..16).rev() {
                let bit = (frame >> i) & 1;
                if bit == 1 {
                    self.pin.set_high();
                    Timer::after(Duration::from_nanos(self.t1_high_ns)).await;
                    self.pin.set_low();
                    Timer::after(Duration::from_nanos(self.t1_low_ns)).await;
                } else {
                    self.pin.set_high();
                    Timer::after(Duration::from_nanos(self.t0_high_ns)).await;
                    self.pin.set_low();
                    Timer::after(Duration::from_nanos(self.t0_low_ns)).await;
                }
            }
        }
    }
}
