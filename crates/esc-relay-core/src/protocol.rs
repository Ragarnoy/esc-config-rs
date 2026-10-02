//! USB command protocol between CLI and relay.
//!
//! This defines a simple framed protocol for controlling the relay:
//!
//! ```text
//! Command frame:  [CMD:1] [LEN:1] [DATA:LEN] [CRC:1]
//! Response frame: [STATUS:1] [LEN:1] [DATA:LEN] [CRC:1]
//! ```
//!
//! The CRC is XOR of all preceding bytes.

use heapless::Vec;

/// Maximum payload size for commands/responses.
pub const MAX_PAYLOAD: usize = 64;

/// Command opcodes from CLI to relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u8)]
pub enum CommandOp {
    /// Ping - check relay is alive. Response: Ack.
    Ping = 0x00,

    /// Get relay version/info. Response: version string.
    Info = 0x01,

    /// Enter passthrough mode (transparent USB <-> UART bridge).
    /// The relay will forward all subsequent data to/from UART.
    EnterPassthrough = 0x20,

    /// Exit passthrough mode and return to command mode.
    ExitPassthrough = 0x21,

    /// Reset relay to idle state.
    Reset = 0xFF,
}

impl CommandOp {
    /// Try to parse command opcode from byte.
    pub const fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x00 => Some(Self::Ping),
            0x01 => Some(Self::Info),
            0x20 => Some(Self::EnterPassthrough),
            0x21 => Some(Self::ExitPassthrough),
            0xFF => Some(Self::Reset),
            _ => None,
        }
    }
}

/// Response status codes from relay to CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u8)]
pub enum Status {
    /// Command succeeded.
    Ok = 0x00,

    /// Unknown command.
    UnknownCommand = 0x01,

    /// Invalid payload length or format.
    InvalidPayload = 0x02,

    /// Operation failed.
    Failed = 0x03,

    /// Relay is in wrong state for this command.
    InvalidState = 0x04,

    /// CRC mismatch.
    CrcError = 0x05,
}

/// Parsed command from CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Command {
    pub op: CommandOp,
    pub payload: Vec<u8, MAX_PAYLOAD>,
}

impl Command {
    /// Parse a command from raw bytes.
    ///
    /// Expected format: [CMD:1] [LEN:1] [DATA:LEN] [CRC:1]
    pub fn parse(data: &[u8]) -> Result<Self, Status> {
        if data.len() < 3 {
            return Err(Status::InvalidPayload);
        }

        let op_byte = data[0];
        let len = data[1] as usize;

        // Check we have enough bytes
        if data.len() < 3 + len {
            return Err(Status::InvalidPayload);
        }

        // Verify CRC
        let crc_idx = 2 + len;
        let expected_crc = compute_crc(&data[..crc_idx]);
        if data[crc_idx] != expected_crc {
            return Err(Status::CrcError);
        }

        let op = CommandOp::from_byte(op_byte).ok_or(Status::UnknownCommand)?;

        let mut payload = Vec::new();
        payload
            .extend_from_slice(&data[2..2 + len])
            .map_err(|_| Status::InvalidPayload)?;

        Ok(Self { op, payload })
    }

    /// Get the minimum frame size for this command (header + crc, no payload).
    pub const fn min_frame_size() -> usize {
        3 // CMD + LEN + CRC
    }
}

/// Response to send back to CLI.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Response {
    pub status: Status,
    pub payload: Vec<u8, MAX_PAYLOAD>,
}

impl Response {
    /// Create a success response with no payload.
    pub const fn ok() -> Self {
        Self {
            status: Status::Ok,
            payload: Vec::new(),
        }
    }

    /// Create a success response with payload.
    pub fn ok_with_data(data: &[u8]) -> Self {
        let mut payload = Vec::new();
        let _ = payload.extend_from_slice(data);
        Self {
            status: Status::Ok,
            payload,
        }
    }

    /// Create an error response.
    pub const fn error(status: Status) -> Self {
        Self {
            status,
            payload: Vec::new(),
        }
    }

    /// Serialize response to bytes.
    ///
    /// Format: [STATUS:1] [LEN:1] [DATA:LEN] [CRC:1]
    pub fn serialize(&self, buf: &mut [u8]) -> usize {
        let len = self.payload.len();
        if buf.len() < 3 + len {
            return 0;
        }

        buf[0] = self.status as u8;
        // Safe: MAX_PAYLOAD is 64, fits in u8
        buf[1] = u8::try_from(len).unwrap_or(u8::MAX);
        buf[2..2 + len].copy_from_slice(&self.payload);
        buf[2 + len] = compute_crc(&buf[..2 + len]);

        3 + len
    }
}

/// Compute XOR checksum of data.
fn compute_crc(data: &[u8]) -> u8 {
    data.iter().fold(0, |acc, &b| acc ^ b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ping_command() {
        // CMD=0x00, LEN=0x00, CRC=0x00
        let data = [0x00, 0x00, 0x00];
        let cmd = Command::parse(&data).unwrap();
        assert_eq!(cmd.op, CommandOp::Ping);
        assert!(cmd.payload.is_empty());
    }

    #[test]
    fn parse_bad_crc() {
        let data = [0x00, 0x00, 0xFF]; // Wrong CRC
        assert_eq!(Command::parse(&data), Err(Status::CrcError));
    }

    #[test]
    fn serialize_ok_response() {
        let resp = Response::ok();
        let mut buf = [0u8; 16];
        let len = resp.serialize(&mut buf);

        assert_eq!(len, 3);
        assert_eq!(buf[0], 0x00); // Status::Ok
        assert_eq!(buf[1], 0x00); // Length
        assert_eq!(buf[2], 0x00); // CRC
    }

    #[test]
    fn serialize_response_with_data() {
        let resp = Response::ok_with_data(b"v1.0");
        let mut buf = [0u8; 16];
        let len = resp.serialize(&mut buf);

        assert_eq!(len, 7); // STATUS + LEN + 4 data + CRC
        assert_eq!(buf[0], 0x00); // Status::Ok
        assert_eq!(buf[1], 0x04); // Length
        assert_eq!(&buf[2..6], b"v1.0");
    }
}
