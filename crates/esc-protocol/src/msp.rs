//! MSP (MultiWii Serial Protocol) packet building and parsing.
//!
//! MSP is used to communicate with Betaflight/INAV flight controllers.
//! This module provides packet builders - no I/O, caller handles serial.
//!
//! # Packet Format
//!
//! ```text
//! Request:  [$] [M] [<] [LEN] [CMD] [PAYLOAD...] [CHECKSUM]
//! Response: [$] [M] [>] [LEN] [CMD] [PAYLOAD...] [CHECKSUM]
//! ```
//!
//! Checksum = XOR of LEN + CMD + all payload bytes

/// MSP request header: "$M<"
pub const MSP_HEADER_REQUEST: [u8; 3] = [0x24, 0x4D, 0x3C];

/// MSP response header: "$M>"
pub const MSP_HEADER_RESPONSE: [u8; 3] = [0x24, 0x4D, 0x3E];

/// Minimum MSP packet size: header(3) + len(1) + cmd(1) + checksum(1)
pub const MSP_MIN_PACKET_SIZE: usize = 6;

/// MSP command codes used for ESC configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MspCommand {
    /// Query API version - verifies Betaflight connection
    ApiVersion = 0x68,

    /// Reboot flight controller (critical after 4-way mode!)
    SetReboot = 0x44,

    /// Set motor throttle values (16 bytes payload)
    SetMotor = 0xD6,

    /// Enter Four-Way passthrough mode
    Set4WayIf = 0xF5,
}

impl MspCommand {
    /// Convert to raw byte value.
    #[inline]
    pub const fn as_byte(self) -> u8 {
        self as u8
    }

    /// Try to parse from byte.
    pub const fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x68 => Some(Self::ApiVersion),
            0x44 => Some(Self::SetReboot),
            0xD6 => Some(Self::SetMotor),
            0xF5 => Some(Self::Set4WayIf),
            _ => None,
        }
    }
}

/// Compute MSP checksum (XOR of all bytes from LEN onwards).
#[inline]
pub fn msp_checksum(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |acc, &b| acc ^ b)
}

/// Build MSP_API_VERSION request (no payload).
///
/// Returns 6-byte packet: "$M<" + len(0) + cmd(0x68) + checksum
#[inline]
pub const fn build_api_version_packet() -> [u8; 6] {
    [
        0x24, 0x4D, 0x3C, // "$M<"
        0x00,             // LEN = 0
        0x68,             // CMD = ApiVersion
        0x68,             // CHECKSUM = 0x00 ^ 0x68
    ]
}

/// Build MSP_SET_4WAY_IF request (no payload).
///
/// Returns 6-byte packet: "$M<" + len(0) + cmd(0xF5) + checksum
#[inline]
pub const fn build_set_4way_if_packet() -> [u8; 6] {
    [
        0x24, 0x4D, 0x3C, // "$M<"
        0x00,             // LEN = 0
        0xF5,             // CMD = Set4WayIf
        0xF5,             // CHECKSUM = 0x00 ^ 0xF5
    ]
}

/// Build MSP_SET_REBOOT request (no payload).
///
/// Returns 6-byte packet: "$M<" + len(0) + cmd(0x44) + checksum
///
/// **Important:** Always reboot the FC after exiting 4-way mode,
/// otherwise it may hold the last throttle value!
#[inline]
pub const fn build_reboot_packet() -> [u8; 6] {
    [
        0x24, 0x4D, 0x3C, // "$M<"
        0x00,             // LEN = 0
        0x44,             // CMD = SetReboot
        0x44,             // CHECKSUM = 0x00 ^ 0x44
    ]
}

/// Build MSP_SET_MOTOR packet with 4 throttle values.
///
/// Throttle values are clamped to 1000-2000 range.
/// Returns 22-byte packet: header(3) + len(1) + cmd(1) + data(16) + checksum(1)
pub fn build_motor_packet(throttles: [u16; 4]) -> [u8; 22] {
    let mut packet = [0u8; 22];

    // Header
    packet[0..3].copy_from_slice(&MSP_HEADER_REQUEST);
    packet[3] = 16; // LEN = 16 bytes payload
    packet[4] = MspCommand::SetMotor.as_byte();

    // Payload: 4 motors as u16 LE + 8 bytes padding (for 8 motor support)
    for (i, &throttle) in throttles.iter().enumerate() {
        let clamped = throttle.clamp(1000, 2000);
        let offset = 5 + i * 2;
        packet[offset] = clamped as u8;
        packet[offset + 1] = (clamped >> 8) as u8;
    }
    // Bytes 13-20 remain 0 (padding for motors 5-8)

    // Checksum: XOR of bytes 3..21 (LEN + CMD + PAYLOAD)
    packet[21] = msp_checksum(&packet[3..21]);

    packet
}

/// Parsed MSP response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MspResponse<'a> {
    /// Command code echoed back
    pub command: u8,
    /// Response payload (borrowed from input buffer)
    pub payload: &'a [u8],
}

/// MSP parsing error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MspError {
    /// Response too short
    TooShort,
    /// Invalid header (not "$M>")
    InvalidHeader,
    /// Checksum mismatch
    ChecksumMismatch,
    /// Buffer too small for declared length
    IncompletePayload,
    /// Command mismatch (for specific parsers)
    CommandMismatch { expected: u8, actual: u8 },
}

impl core::fmt::Display for MspError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort => write!(f, "response too short"),
            Self::InvalidHeader => write!(f, "invalid MSP header"),
            Self::ChecksumMismatch => write!(f, "checksum mismatch"),
            Self::IncompletePayload => write!(f, "incomplete payload"),
            Self::CommandMismatch { expected, actual } => {
                write!(f, "command mismatch: expected 0x{:02X}, got 0x{:02X}", expected, actual)
            }
        }
    }
}

/// Parse MSP response from raw bytes.
///
/// Returns the command code and payload slice if valid.
pub fn parse_msp_response(data: &[u8]) -> Result<MspResponse<'_>, MspError> {
    if data.len() < MSP_MIN_PACKET_SIZE {
        return Err(MspError::TooShort);
    }

    // Check header "$M>"
    if data[0..3] != MSP_HEADER_RESPONSE {
        return Err(MspError::InvalidHeader);
    }

    let len = data[3] as usize;
    let cmd = data[4];

    // Check we have enough bytes: header(3) + len(1) + cmd(1) + payload(len) + checksum(1)
    let expected_len = 6 + len;
    if data.len() < expected_len {
        return Err(MspError::IncompletePayload);
    }

    // Verify checksum (XOR of len + cmd + payload)
    let checksum_idx = 5 + len;
    let calculated = msp_checksum(&data[3..checksum_idx]);
    if data[checksum_idx] != calculated {
        return Err(MspError::ChecksumMismatch);
    }

    let payload = &data[5..5 + len];

    Ok(MspResponse { command: cmd, payload })
}

/// Parse MSP_SET_4WAY_IF response to extract motor count.
///
/// The response payload is a single byte with the number of motors.
pub fn parse_4way_if_response(data: &[u8]) -> Result<u8, MspError> {
    let response = parse_msp_response(data)?;

    if response.command != MspCommand::Set4WayIf.as_byte() {
        return Err(MspError::CommandMismatch {
            expected: MspCommand::Set4WayIf.as_byte(),
            actual: response.command,
        });
    }

    if response.payload.is_empty() {
        return Err(MspError::IncompletePayload);
    }

    Ok(response.payload[0])
}

/// Parse MSP_API_VERSION response.
///
/// Returns (protocol_version, api_major, api_minor) if valid.
pub fn parse_api_version_response(data: &[u8]) -> Result<(u8, u8, u8), MspError> {
    let response = parse_msp_response(data)?;

    if response.command != MspCommand::ApiVersion.as_byte() {
        return Err(MspError::CommandMismatch {
            expected: MspCommand::ApiVersion.as_byte(),
            actual: response.command,
        });
    }

    if response.payload.len() < 3 {
        return Err(MspError::IncompletePayload);
    }

    Ok((response.payload[0], response.payload[1], response.payload[2]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_version_packet_format() {
        let packet = build_api_version_packet();
        assert_eq!(&packet[0..3], b"$M<");
        assert_eq!(packet[3], 0x00); // LEN
        assert_eq!(packet[4], 0x68); // CMD
        assert_eq!(packet[5], 0x68); // CHECKSUM
    }

    #[test]
    fn set_4way_if_packet_format() {
        let packet = build_set_4way_if_packet();
        assert_eq!(&packet[0..3], b"$M<");
        assert_eq!(packet[3], 0x00); // LEN
        assert_eq!(packet[4], 0xF5); // CMD
        assert_eq!(packet[5], 0xF5); // CHECKSUM
    }

    #[test]
    fn reboot_packet_format() {
        let packet = build_reboot_packet();
        assert_eq!(&packet[0..3], b"$M<");
        assert_eq!(packet[3], 0x00); // LEN
        assert_eq!(packet[4], 0x44); // CMD
        assert_eq!(packet[5], 0x44); // CHECKSUM
    }

    #[test]
    fn msp_checksum_calculation() {
        // LEN=0, CMD=0x68 -> 0x00 ^ 0x68 = 0x68
        assert_eq!(msp_checksum(&[0x00, 0x68]), 0x68);

        // LEN=0, CMD=0xF5 -> 0x00 ^ 0xF5 = 0xF5
        assert_eq!(msp_checksum(&[0x00, 0xF5]), 0xF5);

        // LEN=1, CMD=0x68, PAYLOAD=0x04 -> 0x01 ^ 0x68 ^ 0x04 = 0x6D
        assert_eq!(msp_checksum(&[0x01, 0x68, 0x04]), 0x6D);
    }

    #[test]
    fn motor_packet_format() {
        let packet = build_motor_packet([1500, 1000, 2000, 1750]);
        assert_eq!(&packet[0..3], b"$M<");
        assert_eq!(packet[3], 16); // LEN = 16

        // Motor 0: 1500 = 0x05DC (LE: DC 05)
        assert_eq!(packet[5], 0xDC);
        assert_eq!(packet[6], 0x05);

        // Motor 1: 1000 = 0x03E8 (LE: E8 03)
        assert_eq!(packet[7], 0xE8);
        assert_eq!(packet[8], 0x03);

        // Motor 2: 2000 = 0x07D0 (LE: D0 07)
        assert_eq!(packet[9], 0xD0);
        assert_eq!(packet[10], 0x07);

        // Motor 3: 1750 = 0x06D6 (LE: D6 06)
        assert_eq!(packet[11], 0xD6);
        assert_eq!(packet[12], 0x06);
    }

    #[test]
    fn parse_valid_response() {
        // "$M>" + LEN=1 + CMD=0x68 + PAYLOAD=0x04 + CHECKSUM
        let checksum = 0x01 ^ 0x68 ^ 0x04;
        let data = [0x24, 0x4D, 0x3E, 0x01, 0x68, 0x04, checksum];
        let response = parse_msp_response(&data).unwrap();
        assert_eq!(response.command, 0x68);
        assert_eq!(response.payload, &[0x04]);
    }

    #[test]
    fn parse_response_too_short() {
        let data = [0x24, 0x4D, 0x3E, 0x00, 0x68]; // Missing checksum
        assert_eq!(parse_msp_response(&data), Err(MspError::TooShort));
    }

    #[test]
    fn parse_bad_header() {
        let data = [0x24, 0x4D, 0x3C, 0x00, 0x68, 0x68]; // Request header instead of response
        assert_eq!(parse_msp_response(&data), Err(MspError::InvalidHeader));
    }

    #[test]
    fn parse_bad_checksum() {
        let data = [0x24, 0x4D, 0x3E, 0x00, 0x68, 0xFF]; // Wrong checksum
        assert_eq!(parse_msp_response(&data), Err(MspError::ChecksumMismatch));
    }

    #[test]
    fn parse_4way_if_response_valid() {
        // Response with motor count = 4
        let checksum = 0x01 ^ 0xF5 ^ 0x04;
        let data = [0x24, 0x4D, 0x3E, 0x01, 0xF5, 0x04, checksum];
        let motor_count = parse_4way_if_response(&data).unwrap();
        assert_eq!(motor_count, 4);
    }

    #[test]
    fn parse_api_version_response_valid() {
        // Response with protocol=0, major=1, minor=45
        let checksum = 0x03 ^ 0x68 ^ 0x00 ^ 0x01 ^ 0x2D;
        let data = [0x24, 0x4D, 0x3E, 0x03, 0x68, 0x00, 0x01, 0x2D, checksum];
        let (proto, major, minor) = parse_api_version_response(&data).unwrap();
        assert_eq!(proto, 0);
        assert_eq!(major, 1);
        assert_eq!(minor, 45);
    }

    #[test]
    fn command_byte_conversion() {
        assert_eq!(MspCommand::ApiVersion.as_byte(), 0x68);
        assert_eq!(MspCommand::Set4WayIf.as_byte(), 0xF5);
        assert_eq!(MspCommand::from_byte(0x68), Some(MspCommand::ApiVersion));
        assert_eq!(MspCommand::from_byte(0x99), None);
    }
}
