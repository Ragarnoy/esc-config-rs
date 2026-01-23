//! Bootloader protocol state machine.
//!
//! Handles the protocol flow for BLHeli_S and AM32 ESCs.
//! This is pure logic - no I/O. The caller handles actual communication.
//!
//! **Supported:**
//! - BLHeli_S (SIL/Silabs bootloader)
//! - AM32 (ARM bootloader)
//!
//! **NOT Supported:**
//! - BLHeli_32

use crate::device::{classify_device, DeviceInfo};
use crate::packet::{
    build_commit_packet, build_connect_packet, build_disconnect_packet, build_erase_page_packet,
    build_keepalive_packet, build_read_16_packet, build_read_256_packet, build_set_address_packet,
    build_write_header_packet, build_write_page_packet, compute_data_crc, is_ack, is_keepalive_ack,
    parse_connect_response, ADDR_CONFIG,
};

/// Protocol state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum State {
    /// Not connected to ESC.
    #[default]
    Disconnected,
    /// Connected, can send commands.
    Connected,
    /// Address set, ready for read/write.
    AddressSet { address: u16 },
}

/// Bootloader session managing protocol state.
#[derive(Debug)]
pub struct Session {
    state: State,
    device: Option<DeviceInfo>,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    /// Create a new disconnected session.
    #[inline]
    pub const fn new() -> Self {
        Self {
            state: State::Disconnected,
            device: None,
        }
    }

    /// Get current state.
    #[inline]
    pub const fn state(&self) -> State {
        self.state
    }

    /// Get device info (available after successful connect).
    #[inline]
    pub const fn device(&self) -> Option<&DeviceInfo> {
        self.device.as_ref()
    }

    /// Check if connected.
    #[inline]
    pub const fn is_connected(&self) -> bool {
        !matches!(self.state, State::Disconnected)
    }

    // ========================================================================
    // Command Generation
    // ========================================================================

    /// Get connect packet bytes (Direct protocol).
    ///
    /// Call this when `state == Disconnected`.
    /// Returns 21 bytes: 12x 0x00 + 0x0D + "BLHeli" + CRC
    pub fn connect_command(&self) -> Result<[u8; 21], ProtocolError> {
        if self.state != State::Disconnected {
            return Err(ProtocolError::InvalidState {
                expected: State::Disconnected,
                actual: self.state,
            });
        }
        Ok(build_connect_packet())
    }

    /// Get keepalive packet bytes.
    ///
    /// Call this when connected to prevent timeout.
    pub fn keepalive_command(&self) -> Result<[u8; 4], ProtocolError> {
        if !self.is_connected() {
            return Err(ProtocolError::NotConnected);
        }
        Ok(build_keepalive_packet())
    }

    /// Get disconnect packet bytes.
    pub fn disconnect_command(&self) -> Result<[u8; 4], ProtocolError> {
        if !self.is_connected() {
            return Err(ProtocolError::NotConnected);
        }
        Ok(build_disconnect_packet())
    }

    /// Get set-address packet bytes.
    pub fn set_address_command(&self, address: u16) -> Result<[u8; 6], ProtocolError> {
        if !self.is_connected() {
            return Err(ProtocolError::NotConnected);
        }
        Ok(build_set_address_packet(address))
    }

    /// Get read-256 packet bytes.
    ///
    /// Must have address set first.
    pub fn read_256_command(&self) -> Result<[u8; 4], ProtocolError> {
        match self.state {
            State::AddressSet { .. } => Ok(build_read_256_packet()),
            State::Connected => Err(ProtocolError::AddressNotSet),
            State::Disconnected => Err(ProtocolError::NotConnected),
        }
    }

    /// Get read-16 packet bytes.
    ///
    /// Must have address set first.
    pub fn read_16_command(&self) -> Result<[u8; 4], ProtocolError> {
        match self.state {
            State::AddressSet { .. } => Ok(build_read_16_packet()),
            State::Connected => Err(ProtocolError::AddressNotSet),
            State::Disconnected => Err(ProtocolError::NotConnected),
        }
    }

    /// Get write header packet bytes (long format for BLHeli_S).
    ///
    /// Must have address set first. After sending this, send 256 data bytes + 2 CRC bytes.
    /// Note: The ESC does NOT ACK the header - it waits for the full data + CRC before responding.
    pub fn write_header_command(&self) -> Result<[u8; 6], ProtocolError> {
        match self.state {
            State::AddressSet { .. } => Ok(build_write_header_packet()),
            State::Connected => Err(ProtocolError::AddressNotSet),
            State::Disconnected => Err(ProtocolError::NotConnected),
        }
    }

    /// Get write page packet bytes (short format for AM32).
    ///
    /// Must have address set first. After sending this, wait for ACK, then send data + CRC.
    pub fn write_page_command(&self) -> Result<[u8; 4], ProtocolError> {
        match self.state {
            State::AddressSet { .. } => Ok(build_write_page_packet()),
            State::Connected => Err(ProtocolError::AddressNotSet),
            State::Disconnected => Err(ProtocolError::NotConnected),
        }
    }

    /// Compute CRC for write data.
    #[inline]
    pub fn compute_write_crc(data: &[u8]) -> [u8; 2] {
        compute_data_crc(data)
    }

    /// Get erase page packet bytes.
    ///
    /// Erases the flash page at the currently set address.
    /// Must have address set first.
    pub fn erase_page_command(&self) -> Result<[u8; 4], ProtocolError> {
        match self.state {
            State::AddressSet { .. } => Ok(build_erase_page_packet()),
            State::Connected => Err(ProtocolError::AddressNotSet),
            State::Disconnected => Err(ProtocolError::NotConnected),
        }
    }

    /// Get commit packet bytes.
    ///
    /// Call after write to commit changes to flash.
    pub fn commit_command(&self) -> Result<[u8; 4], ProtocolError> {
        if !self.is_connected() {
            return Err(ProtocolError::NotConnected);
        }
        Ok(build_commit_packet())
    }

    // ========================================================================
    // Response Handling
    // ========================================================================

    /// Process connect response.
    ///
    /// Updates state to `Connected` on success.
    pub fn handle_connect_response(&mut self, data: &[u8]) -> Result<&DeviceInfo, ProtocolError> {
        if self.state != State::Disconnected {
            return Err(ProtocolError::InvalidState {
                expected: State::Disconnected,
                actual: self.state,
            });
        }

        let response = parse_connect_response(data).ok_or(ProtocolError::MalformedResponse)?;

        let device_info = classify_device(response.device_id);

        // Check if device is supported
        if !device_info.is_supported() {
            return Err(ProtocolError::UnsupportedDevice {
                device_id: response.device_id,
            });
        }

        self.device = Some(device_info);
        self.state = State::Connected;

        Ok(self.device.as_ref().unwrap())
    }

    /// Process keepalive response (single byte).
    pub fn handle_keepalive_response(&self, byte: u8) -> Result<(), ProtocolError> {
        if !self.is_connected() {
            return Err(ProtocolError::NotConnected);
        }
        if is_keepalive_ack(byte) {
            Ok(())
        } else {
            Err(ProtocolError::UnexpectedResponse { byte })
        }
    }

    /// Process ACK response (single byte).
    pub fn handle_ack_response(&self, byte: u8) -> Result<(), ProtocolError> {
        if is_ack(byte) {
            Ok(())
        } else {
            Err(ProtocolError::UnexpectedResponse { byte })
        }
    }

    /// Process set-address ACK and update state.
    pub fn handle_set_address_response(
        &mut self,
        byte: u8,
        address: u16,
    ) -> Result<(), ProtocolError> {
        if !self.is_connected() {
            return Err(ProtocolError::NotConnected);
        }
        if is_ack(byte) {
            self.state = State::AddressSet { address };
            Ok(())
        } else {
            Err(ProtocolError::UnexpectedResponse { byte })
        }
    }

    /// Process disconnect ACK and update state.
    ///
    /// Accepts both ACK (0x30) and KEEPALIVE_ACK (0xC1) since some ESCs
    /// (particularly AM32) may respond with either.
    pub fn handle_disconnect_response(&mut self, byte: u8) -> Result<(), ProtocolError> {
        if is_ack(byte) || is_keepalive_ack(byte) {
            self.state = State::Disconnected;
            self.device = None;
            Ok(())
        } else {
            Err(ProtocolError::UnexpectedResponse { byte })
        }
    }

    /// Validate read response structure.
    ///
    /// Returns the data portion (without trailing CRC and ACK).
    /// For 256-byte read: expects 256 data + 2 CRC + 1 ACK = 259 bytes.
    pub fn validate_read_response<'a>(&self, data: &'a [u8], expected_len: usize) -> Result<&'a [u8], ProtocolError> {
        // Expected: data + 2 CRC + 1 ACK
        let total_expected = expected_len + 3;

        if data.len() < total_expected {
            return Err(ProtocolError::ResponseTooShort {
                expected: total_expected,
                actual: data.len(),
            });
        }

        // Check ACK at end
        if !is_ack(data[expected_len + 2]) {
            return Err(ProtocolError::UnexpectedResponse {
                byte: data[expected_len + 2],
            });
        }

        // TODO: Verify CRC of data portion
        // For now, trust the data

        Ok(&data[..expected_len])
    }

    /// Force reset to disconnected state.
    ///
    /// Use after communication errors or timeouts.
    pub fn reset(&mut self) {
        self.state = State::Disconnected;
        self.device = None;
    }
}

/// Protocol errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolError {
    /// Wrong state for this operation.
    InvalidState { expected: State, actual: State },
    /// Not connected to ESC.
    NotConnected,
    /// Address not set before read/write.
    AddressNotSet,
    /// Response parsing failed.
    MalformedResponse,
    /// Unexpected response byte.
    UnexpectedResponse { byte: u8 },
    /// Response too short.
    ResponseTooShort { expected: usize, actual: usize },
    /// Device not supported.
    UnsupportedDevice { device_id: u16 },
}

impl core::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidState { expected, actual } => {
                write!(f, "invalid state: expected {:?}, got {:?}", expected, actual)
            }
            Self::NotConnected => write!(f, "not connected"),
            Self::AddressNotSet => write!(f, "address not set"),
            Self::MalformedResponse => write!(f, "malformed response"),
            Self::UnexpectedResponse { byte } => {
                write!(f, "unexpected response: 0x{:02X}", byte)
            }
            Self::ResponseTooShort { expected, actual } => {
                write!(f, "response too short: expected {}, got {}", expected, actual)
            }
            Self::UnsupportedDevice { device_id } => {
                write!(f, "unsupported device: 0x{:04X}", device_id)
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ProtocolError {}

// ============================================================================
// High-level Operations
// ============================================================================

/// Read configuration block (256 bytes) from ESC.
///
/// This is a helper that generates the sequence of commands needed.
/// The caller must handle actual I/O.
pub struct ReadConfigOp {
    step: ReadConfigStep,
}

impl Default for ReadConfigOp {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadConfigStep {
    SetAddress,
    Read,
    Done,
}

impl ReadConfigOp {
    /// Start a config read operation.
    pub const fn new() -> Self {
        Self {
            step: ReadConfigStep::SetAddress,
        }
    }

    /// Get the next command to send.
    ///
    /// Returns `None` when operation is complete.
    pub fn next_command(&self) -> Option<ReadConfigCommand> {
        match self.step {
            ReadConfigStep::SetAddress => Some(ReadConfigCommand::SetAddress(ADDR_CONFIG)),
            ReadConfigStep::Read => Some(ReadConfigCommand::Read256),
            ReadConfigStep::Done => None,
        }
    }

    /// Advance to next step after successful response.
    pub fn advance(&mut self) {
        self.step = match self.step {
            ReadConfigStep::SetAddress => ReadConfigStep::Read,
            ReadConfigStep::Read => ReadConfigStep::Done,
            ReadConfigStep::Done => ReadConfigStep::Done,
        };
    }

    /// Check if operation is complete.
    pub const fn is_done(&self) -> bool {
        matches!(self.step, ReadConfigStep::Done)
    }
}

/// Commands for read config operation.
#[derive(Debug, Clone, Copy)]
pub enum ReadConfigCommand {
    SetAddress(u16),
    Read256,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::FirmwareType;

    #[test]
    fn session_starts_disconnected() {
        let session = Session::new();
        assert_eq!(session.state(), State::Disconnected);
        assert!(!session.is_connected());
        assert!(session.device().is_none());
    }

    #[test]
    fn connect_command_only_when_disconnected() {
        let mut session = Session::new();
        assert!(session.connect_command().is_ok());

        // Simulate connection
        let response = [0x34, 0x37, 0x31, 0x90, 0xF3, 0x06, 0x07, 0x04, 0x30];
        session.handle_connect_response(&response).unwrap();

        // Now connect should fail
        assert!(matches!(
            session.connect_command(),
            Err(ProtocolError::InvalidState { .. })
        ));
    }

    #[test]
    fn handle_connect_response_success() {
        let mut session = Session::new();

        // Valid response with SIL device ID 0xF390
        let response = [0x34, 0x37, 0x31, 0x90, 0xF3, 0x06, 0x07, 0x04, 0x30];
        let device = session.handle_connect_response(&response).unwrap();

        assert_eq!(device.device_id, 0xF390);
        assert_eq!(device.firmware, Some(FirmwareType::BlheliS));
        assert_eq!(session.state(), State::Connected);
    }

    #[test]
    fn handle_connect_response_unsupported() {
        let mut session = Session::new();

        // ATmega device - unsupported
        let response = [0x34, 0x37, 0x31, 0x07, 0x93, 0x06, 0x07, 0x04, 0x30];
        let result = session.handle_connect_response(&response);

        assert!(matches!(
            result,
            Err(ProtocolError::UnsupportedDevice { .. })
        ));
        assert_eq!(session.state(), State::Disconnected);
    }

    #[test]
    fn keepalive_requires_connection() {
        let session = Session::new();
        assert!(matches!(
            session.keepalive_command(),
            Err(ProtocolError::NotConnected)
        ));
    }

    #[test]
    fn set_address_updates_state() {
        let mut session = Session::new();

        // Connect first
        let response = [0x34, 0x37, 0x31, 0x90, 0xF3, 0x06, 0x07, 0x04, 0x30];
        session.handle_connect_response(&response).unwrap();

        // Set address command should work
        let cmd = session.set_address_command(0x7C00);
        assert!(cmd.is_ok());

        // Handle ACK
        session.handle_set_address_response(0x30, 0x7C00).unwrap();
        assert_eq!(session.state(), State::AddressSet { address: 0x7C00 });
    }

    #[test]
    fn read_requires_address_set() {
        let mut session = Session::new();

        // Connect
        let response = [0x34, 0x37, 0x31, 0x90, 0xF3, 0x06, 0x07, 0x04, 0x30];
        session.handle_connect_response(&response).unwrap();

        // Read should fail - no address set
        assert!(matches!(
            session.read_256_command(),
            Err(ProtocolError::AddressNotSet)
        ));

        // Set address
        session.handle_set_address_response(0x30, 0x7C00).unwrap();

        // Now read should work
        assert!(session.read_256_command().is_ok());
    }

    #[test]
    fn disconnect_resets_state() {
        let mut session = Session::new();

        // Connect
        let response = [0x34, 0x37, 0x31, 0x90, 0xF3, 0x06, 0x07, 0x04, 0x30];
        session.handle_connect_response(&response).unwrap();
        assert!(session.is_connected());

        // Disconnect
        session.handle_disconnect_response(0x30).unwrap();
        assert_eq!(session.state(), State::Disconnected);
        assert!(session.device().is_none());
    }

    #[test]
    fn validate_read_response_success() {
        let mut session = Session::new();

        // Setup: connect and set address
        let response = [0x34, 0x37, 0x31, 0x90, 0xF3, 0x06, 0x07, 0x04, 0x30];
        session.handle_connect_response(&response).unwrap();
        session.handle_set_address_response(0x30, 0x7C00).unwrap();

        // 16 bytes data + 2 CRC + 1 ACK
        let mut read_response = [0u8; 19];
        read_response[0..16].fill(0xAB); // data
        read_response[16] = 0x12; // CRC LSB
        read_response[17] = 0x34; // CRC MSB
        read_response[18] = 0x30; // ACK

        let data = session.validate_read_response(&read_response, 16).unwrap();
        assert_eq!(data.len(), 16);
        assert!(data.iter().all(|&b| b == 0xAB));
    }

    #[test]
    fn validate_read_response_no_ack() {
        let session = Session::new();

        let mut read_response = [0u8; 19];
        read_response[18] = 0x00; // Wrong ACK

        assert!(matches!(
            session.validate_read_response(&read_response, 16),
            Err(ProtocolError::UnexpectedResponse { byte: 0x00 })
        ));
    }

    #[test]
    fn read_config_op_sequence() {
        let op = ReadConfigOp::new();

        assert!(!op.is_done());
        assert!(matches!(
            op.next_command(),
            Some(ReadConfigCommand::SetAddress(0x7C00))
        ));
    }

    #[test]
    fn reset_clears_state() {
        let mut session = Session::new();

        // Connect
        let response = [0x34, 0x37, 0x31, 0x90, 0xF3, 0x06, 0x07, 0x04, 0x30];
        session.handle_connect_response(&response).unwrap();
        session.handle_set_address_response(0x30, 0x7C00).unwrap();

        assert!(session.is_connected());
        assert!(session.device().is_some());

        // Reset
        session.reset();

        assert_eq!(session.state(), State::Disconnected);
        assert!(session.device().is_none());
    }
}
