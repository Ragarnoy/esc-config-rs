//! DShot frame encoding/decoding.
//!
//! DShot frame layout (16 bits):
//! ```text
//! ┌─────────────────┬─────┬───────┐
//! │ Throttle (11b)  │ TLM │ CRC-4 │
//! │ bits 15-5       │ b4  │ 3-0   │
//! └─────────────────┴─────┴───────┘
//! ```
//!
//! - Throttle: 0-47 are special commands, 48-2047 is throttle range
//! - TLM: Telemetry request flag
//! - CRC-4: XOR checksum of the three nibbles

use crate::crc4::{crc4, verify_crc4};

// ============================================================================
// Constants
// ============================================================================

/// Maximum throttle value (11-bit).
pub const THROTTLE_MAX: u16 = 2047;

/// Minimum throttle value (first non-command value).
pub const THROTTLE_MIN: u16 = 48;

/// Number of special commands (0-47).
pub const COMMAND_COUNT: u16 = 48;

/// DShot frame size in bits.
pub const FRAME_BITS: usize = 16;

// ============================================================================
// Special Commands
// ============================================================================

/// DShot special commands (sent as throttle values 0-47).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum Command {
    /// Motor stop (disarm).
    MotorStop = 0,
    /// Beep 1 (low frequency).
    Beep1 = 1,
    /// Beep 2.
    Beep2 = 2,
    /// Beep 3.
    Beep3 = 3,
    /// Beep 4.
    Beep4 = 4,
    /// Beep 5 (high frequency).
    Beep5 = 5,
    /// ESC info request (BLHeli_32).
    EscInfo = 6,
    /// Spin direction 1 (normal).
    SpinDirection1 = 7,
    /// Spin direction 2 (reversed).
    SpinDirection2 = 8,
    /// 3D mode off.
    Mode3dOff = 9,
    /// 3D mode on.
    Mode3dOn = 10,
    /// ESC settings request.
    SettingsRequest = 11,
    /// Save settings.
    SaveSettings = 12,
    /// Extended telemetry enable.
    ExtendedTelemetryEnable = 13,
    /// Extended telemetry disable.
    ExtendedTelemetryDisable = 14,
    /// Spin direction normal.
    SpinDirectionNormal = 20,
    /// Spin direction reversed.
    SpinDirectionReversed = 21,
    /// LED0 on.
    Led0On = 22,
    /// LED1 on.
    Led1On = 23,
    /// LED2 on.
    Led2On = 24,
    /// LED3 on.
    Led3On = 25,
    /// LED0 off.
    Led0Off = 26,
    /// LED1 off.
    Led1Off = 27,
    /// LED2 off.
    Led2Off = 28,
    /// LED3 off.
    Led3Off = 29,
    /// Audio stream mode on/off.
    AudioStreamToggle = 30,
    /// Silent mode on/off.
    SilentModeToggle = 31,
}

impl Command {
    /// Convert command to throttle value.
    #[inline]
    pub const fn as_throttle(self) -> u16 {
        self as u16
    }
}

// ============================================================================
// Frame Encoding
// ============================================================================

/// Encode a DShot frame.
///
/// # Arguments
/// * `throttle` - Throttle value (0-2047) or command (0-47)
/// * `telemetry` - Request telemetry response
///
/// # Returns
/// Complete 16-bit DShot frame with CRC
///
/// # Panics
/// Panics if throttle > 2047
#[inline]
pub const fn encode(throttle: u16, telemetry: bool) -> u16 {
    assert!(throttle <= THROTTLE_MAX, "throttle exceeds maximum");

    // Build 12-bit data: throttle[10:0] << 1 | telemetry
    let data = (throttle << 1) | (telemetry as u16);

    // Build 16-bit frame: data << 4 | crc
    let crc = crc4(data);
    (data << 4) | (crc as u16)
}

/// Encode a DShot command frame.
///
/// Commands should be sent with telemetry=true for 10 times,
/// then with telemetry=false for 10 more times (20 total).
#[inline]
pub const fn encode_command(cmd: Command, telemetry: bool) -> u16 {
    encode(cmd.as_throttle(), telemetry)
}

/// Encode a throttle frame (validates throttle is in valid range).
///
/// # Arguments
/// * `throttle` - Throttle value (48-2047)
/// * `telemetry` - Request telemetry response
///
/// # Returns
/// `None` if throttle is in command range (0-47)
#[inline]
pub const fn encode_throttle(throttle: u16, telemetry: bool) -> Option<u16> {
    if throttle < THROTTLE_MIN {
        return None;
    }
    Some(encode(throttle, telemetry))
}

// ============================================================================
// Frame Decoding
// ============================================================================

/// Decoded DShot frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedFrame {
    /// Throttle value or command (0-2047).
    pub throttle: u16,
    /// Telemetry request flag.
    pub telemetry: bool,
    /// CRC valid flag.
    pub crc_valid: bool,
}

impl DecodedFrame {
    /// Check if this is a command (throttle 0-47).
    #[inline]
    pub const fn is_command(&self) -> bool {
        self.throttle < THROTTLE_MIN
    }

    /// Check if this is a throttle value (48-2047).
    #[inline]
    pub const fn is_throttle(&self) -> bool {
        self.throttle >= THROTTLE_MIN
    }

    /// Get the throttle as a normalized 0.0-1.0 value.
    ///
    /// Returns 0.0 for commands.
    #[inline]
    pub fn throttle_normalized(&self) -> f32 {
        if self.throttle < THROTTLE_MIN {
            return 0.0;
        }
        (self.throttle - THROTTLE_MIN) as f32 / (THROTTLE_MAX - THROTTLE_MIN) as f32
    }
}

/// Decode a DShot frame.
///
/// # Arguments
/// * `frame` - Complete 16-bit DShot frame
///
/// # Returns
/// Decoded frame with throttle, telemetry flag, and CRC validity
#[inline]
pub const fn decode(frame: u16) -> DecodedFrame {
    let data = frame >> 4;
    let throttle = data >> 1;
    let telemetry = (data & 1) != 0;
    let crc_valid = verify_crc4(frame);

    DecodedFrame {
        throttle,
        telemetry,
        crc_valid,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_zero_throttle_no_telemetry() {
        let frame = encode(0, false);
        let decoded = decode(frame);

        assert_eq!(decoded.throttle, 0);
        assert!(!decoded.telemetry);
        assert!(decoded.crc_valid);
    }

    #[test]
    fn encode_zero_throttle_with_telemetry() {
        let frame = encode(0, true);
        let decoded = decode(frame);

        assert_eq!(decoded.throttle, 0);
        assert!(decoded.telemetry);
        assert!(decoded.crc_valid);
    }

    #[test]
    fn encode_max_throttle() {
        let frame = encode(THROTTLE_MAX, false);
        let decoded = decode(frame);

        assert_eq!(decoded.throttle, THROTTLE_MAX);
        assert!(!decoded.telemetry);
        assert!(decoded.crc_valid);
    }

    #[test]
    fn encode_mid_throttle() {
        let frame = encode(1000, true);
        let decoded = decode(frame);

        assert_eq!(decoded.throttle, 1000);
        assert!(decoded.telemetry);
        assert!(decoded.crc_valid);
    }

    #[test]
    fn encode_command_beep() {
        let frame = super::encode_command(Command::Beep1, false);
        let decoded = decode(frame);

        assert_eq!(decoded.throttle, 1);
        assert!(!decoded.telemetry);
        assert!(decoded.crc_valid);
        assert!(decoded.is_command());
    }

    #[test]
    fn encode_throttle_rejects_commands() {
        assert!(encode_throttle(0, false).is_none());
        assert!(encode_throttle(47, false).is_none());
        assert!(encode_throttle(48, false).is_some());
    }

    #[test]
    fn decode_invalid_crc() {
        // Create frame with wrong CRC
        let frame = encode(500, false);
        let corrupted = frame ^ 0x0001; // Flip CRC bit

        let decoded = decode(corrupted);
        assert!(!decoded.crc_valid);
    }

    #[test]
    fn throttle_normalized() {
        let frame = decode(encode(THROTTLE_MIN, false));
        assert!((frame.throttle_normalized() - 0.0).abs() < 0.001);

        let frame = decode(encode(THROTTLE_MAX, false));
        assert!((frame.throttle_normalized() - 1.0).abs() < 0.001);

        let frame = decode(encode(0, false)); // Command
        assert_eq!(frame.throttle_normalized(), 0.0);
    }

    #[test]
    fn is_command_vs_throttle() {
        let cmd_frame = decode(encode(10, false));
        assert!(cmd_frame.is_command());
        assert!(!cmd_frame.is_throttle());

        let thr_frame = decode(encode(500, false));
        assert!(!thr_frame.is_command());
        assert!(thr_frame.is_throttle());
    }

    #[test]
    fn frame_bit_layout() {
        // Verify actual bit positions
        // Throttle=100, Telemetry=true
        // Data = (100 << 1) | 1 = 201 = 0x0C9
        // CRC = crc4(0x0C9) = 0 ^ C ^ 9 = 5
        // Frame = (0x0C9 << 4) | 5 = 0xC95

        let frame = encode(100, true);
        assert_eq!(frame, 0x0C95);

        // Verify extraction
        let data = frame >> 4;
        assert_eq!(data, 0x0C9);
        assert_eq!(data >> 1, 100); // throttle
        assert_eq!(data & 1, 1); // telemetry
    }

    #[test]
    fn encode_is_const() {
        const FRAME: u16 = encode(500, true);
        assert!(decode(FRAME).crc_valid);
    }

    #[test]
    #[should_panic(expected = "throttle exceeds maximum")]
    fn encode_panics_on_invalid_throttle() {
        encode(2048, false);
    }
}
