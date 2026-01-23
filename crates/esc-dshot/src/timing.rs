//! DShot timing constants and validation.
//!
//! DShot uses a fixed-length bit encoding where:
//! - Bit 0: short high pulse (~37% duty)
//! - Bit 1: long high pulse (~75% duty)
//!
//! Supported speeds:
//! - DShot150: 150 kHz (6.67 µs/bit, ~107 µs/frame)
//! - DShot300: 300 kHz (3.33 µs/bit, ~53 µs/frame)
//! - DShot600: 600 kHz (1.67 µs/bit, ~27 µs/frame)
//! - DShot1200: 1200 kHz (0.83 µs/bit, ~13 µs/frame)

/// DShot speed variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speed {
    /// 150 kHz - slowest, most tolerant.
    DShot150,
    /// 300 kHz - common for analog ESCs.
    DShot300,
    /// 600 kHz - common for BLHeli_S/BLHeli_32.
    DShot600,
    /// 1200 kHz - fastest, requires good signal integrity.
    DShot1200,
}

impl Speed {
    /// Get bit rate in Hz.
    #[inline]
    pub const fn bit_rate_hz(self) -> u32 {
        match self {
            Self::DShot150 => DSHOT150_HZ,
            Self::DShot300 => DSHOT300_HZ,
            Self::DShot600 => DSHOT600_HZ,
            Self::DShot1200 => DSHOT1200_HZ,
        }
    }

    /// Get bit period in nanoseconds.
    #[inline]
    pub const fn bit_period_ns(self) -> u32 {
        match self {
            Self::DShot150 => DSHOT150_BIT_NS,
            Self::DShot300 => DSHOT300_BIT_NS,
            Self::DShot600 => DSHOT600_BIT_NS,
            Self::DShot1200 => DSHOT1200_BIT_NS,
        }
    }

    /// Get frame period in nanoseconds.
    #[inline]
    pub const fn frame_period_ns(self) -> u32 {
        self.bit_period_ns() * FRAME_BITS as u32
    }

    /// Get T0H (bit-0 high time) in nanoseconds.
    #[inline]
    pub const fn t0h_ns(self) -> u32 {
        // ~37% duty cycle
        (self.bit_period_ns() * 37) / 100
    }

    /// Get T1H (bit-1 high time) in nanoseconds.
    #[inline]
    pub const fn t1h_ns(self) -> u32 {
        // ~75% duty cycle
        (self.bit_period_ns() * 75) / 100
    }
}

// ============================================================================
// Constants
// ============================================================================

/// DShot frame size in bits.
pub const FRAME_BITS: usize = 16;

/// DShot150 bit rate in Hz.
pub const DSHOT150_HZ: u32 = 150_000;

/// DShot300 bit rate in Hz.
pub const DSHOT300_HZ: u32 = 300_000;

/// DShot600 bit rate in Hz.
pub const DSHOT600_HZ: u32 = 600_000;

/// DShot1200 bit rate in Hz.
pub const DSHOT1200_HZ: u32 = 1_200_000;

// Bit periods in nanoseconds (1_000_000_000 / Hz)
/// DShot150 bit period in nanoseconds.
pub const DSHOT150_BIT_NS: u32 = 6667;
/// DShot300 bit period in nanoseconds.
pub const DSHOT300_BIT_NS: u32 = 3333;
/// DShot600 bit period in nanoseconds.
pub const DSHOT600_BIT_NS: u32 = 1667;
/// DShot1200 bit period in nanoseconds.
pub const DSHOT1200_BIT_NS: u32 = 833;

// Frame periods in microseconds
/// DShot150 frame period in microseconds.
pub const DSHOT150_FRAME_US: u32 = 107;
/// DShot300 frame period in microseconds.
pub const DSHOT300_FRAME_US: u32 = 53;
/// DShot600 frame period in microseconds.
pub const DSHOT600_FRAME_US: u32 = 27;
/// DShot1200 frame period in microseconds.
pub const DSHOT1200_FRAME_US: u32 = 13;

/// Minimum pause between frames in microseconds.
/// The ESC needs time to process the frame before receiving the next.
pub const MIN_FRAME_GAP_US: u32 = 2;

/// Telemetry response time window in microseconds.
/// After sending a frame with telemetry bit set, wait this long for response.
pub const TELEMETRY_RESPONSE_US: u32 = 30;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_bit_rates() {
        assert_eq!(Speed::DShot150.bit_rate_hz(), 150_000);
        assert_eq!(Speed::DShot300.bit_rate_hz(), 300_000);
        assert_eq!(Speed::DShot600.bit_rate_hz(), 600_000);
        assert_eq!(Speed::DShot1200.bit_rate_hz(), 1_200_000);
    }

    #[test]
    fn speed_bit_periods() {
        // Verify bit periods are approximately correct
        assert!(Speed::DShot150.bit_period_ns() > 6600);
        assert!(Speed::DShot150.bit_period_ns() < 6700);

        assert!(Speed::DShot600.bit_period_ns() > 1600);
        assert!(Speed::DShot600.bit_period_ns() < 1700);
    }

    #[test]
    fn speed_frame_periods() {
        // DShot600: 16 bits * 1667ns = 26672ns ≈ 27µs
        let frame_ns = Speed::DShot600.frame_period_ns();
        assert!(frame_ns > 26000);
        assert!(frame_ns < 28000);
    }

    #[test]
    fn pulse_widths() {
        let speed = Speed::DShot600;

        // T0H should be shorter than T1H
        assert!(speed.t0h_ns() < speed.t1h_ns());

        // T0H ~37%, T1H ~75% of bit period
        let bit = speed.bit_period_ns();
        assert!((speed.t0h_ns() as f32 / bit as f32 - 0.37).abs() < 0.02);
        assert!((speed.t1h_ns() as f32 / bit as f32 - 0.75).abs() < 0.02);
    }

    #[test]
    fn frame_bits_constant() {
        assert_eq!(FRAME_BITS, 16);
    }
}
