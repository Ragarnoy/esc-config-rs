//! Four-Way Interface protocol packet building and parsing.
//!
//! Four-Way Interface allows ESC configuration through a Betaflight flight controller.
//! The FC acts as a passthrough between the host and the ESC.
//!
//! # Packet Formats
//!
//! ```text
//! Request:  [CMD] [PARAM] [ADDR_HI] [ADDR_LO] [DATA...] [CRC_LO] [CRC_HI]
//! Response: [CMD] [PARAM] [ADDR_HI] [ADDR_LO] [DATA...] [ACK] [CRC_LO] [CRC_HI]
//! ```
//!
//! - ACK is at position `len - 3`
//! - Uses CRC-16/IBM (same as direct bootloader protocol)

use crate::crc::crc16_ibm_bytes;

/// Minimum Four-Way packet size: cmd(1) + param(1) + addr(2) + crc(2) = 6
pub const FOURWAY_MIN_PACKET_SIZE: usize = 6;

/// Minimum Four-Way response size: cmd(1) + param(1) + addr(2) + ack(1) + crc(2) = 7
pub const FOURWAY_MIN_RESPONSE_SIZE: usize = 7;

/// Maximum write payload size
pub const FOURWAY_MAX_WRITE_SIZE: usize = 256;

/// Four-Way command codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FourWayCommand {
    /// Test if interface is alive
    InterfaceTestAlive = 0x30,
    /// Get protocol version
    ProtocolGetVersion = 0x31,
    /// Get interface name
    InterfaceGetName = 0x32,
    /// Get interface version
    InterfaceGetVersion = 0x33,
    /// Exit Four-Way mode
    InterfaceExit = 0x34,
    /// Reset specific ESC
    DeviceReset = 0x35,
    /// Connect to ESC (enter bootloader)
    DeviceInitFlash = 0x37,
    /// Erase all flash
    DeviceEraseAll = 0x38,
    /// Erase specific page
    DevicePageErase = 0x39,
    /// Read from flash
    DeviceRead = 0x3A,
    /// Write to flash
    DeviceWrite = 0x3B,
    /// C2CK low (SiLabs specific)
    DeviceC2CkLow = 0x3C,
    /// Read from EEPROM
    DeviceReadEeprom = 0x3D,
    /// Write to EEPROM
    DeviceWriteEeprom = 0x3E,
    /// Reset device (alias)
    DeviceReset2 = 0x3F,
}

impl FourWayCommand {
    /// Convert to raw byte value.
    #[inline]
    pub const fn as_byte(self) -> u8 {
        self as u8
    }

    /// Try to parse from byte.
    pub const fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x30 => Some(Self::InterfaceTestAlive),
            0x31 => Some(Self::ProtocolGetVersion),
            0x32 => Some(Self::InterfaceGetName),
            0x33 => Some(Self::InterfaceGetVersion),
            0x34 => Some(Self::InterfaceExit),
            0x35 => Some(Self::DeviceReset),
            0x37 => Some(Self::DeviceInitFlash),
            0x38 => Some(Self::DeviceEraseAll),
            0x39 => Some(Self::DevicePageErase),
            0x3A => Some(Self::DeviceRead),
            0x3B => Some(Self::DeviceWrite),
            0x3C => Some(Self::DeviceC2CkLow),
            0x3D => Some(Self::DeviceReadEeprom),
            0x3E => Some(Self::DeviceWriteEeprom),
            0x3F => Some(Self::DeviceReset2),
            _ => None,
        }
    }
}

/// Four-Way acknowledgment codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FourWayAck {
    /// Success
    Ok = 0x00,
    /// Unknown error
    UnknownError = 0x01,
    /// Invalid command
    InvalidCmd = 0x02,
    /// CRC error
    InvalidCrc = 0x03,
    /// Verification failed
    VerifyError = 0x04,
    /// Device general error
    GeneralError = 0x05,
}

impl FourWayAck {
    /// Try to parse from byte.
    pub const fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x00 => Some(Self::Ok),
            0x01 => Some(Self::UnknownError),
            0x02 => Some(Self::InvalidCmd),
            0x03 => Some(Self::InvalidCrc),
            0x04 => Some(Self::VerifyError),
            0x05 => Some(Self::GeneralError),
            _ => None,
        }
    }

    /// Check if this ACK indicates success.
    #[inline]
    pub const fn is_ok(self) -> bool {
        matches!(self, Self::Ok)
    }
}

impl core::fmt::Display for FourWayAck {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Ok => write!(f, "OK"),
            Self::UnknownError => write!(f, "unknown error"),
            Self::InvalidCmd => write!(f, "invalid command"),
            Self::InvalidCrc => write!(f, "CRC error"),
            Self::VerifyError => write!(f, "verify error"),
            Self::GeneralError => write!(f, "general error"),
        }
    }
}

/// MCU type from DeviceInitFlash response.
///
/// These map to specific ARM MCUs with different page sizes and address handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FourWayMcuType {
    /// STM32F051 - 1KB page, EEPROM at 0x7C00, no address division
    Stm32F051 = 0x1F,
    /// STM32G071 - 2KB page, EEPROM at 0x7E00, addresses divided by 4
    Stm32G071 = 0x2B,
    /// STM32F3xx - 2KB page, EEPROM at 0xF800, no address division
    Stm32F3 = 0x35,
}

impl FourWayMcuType {
    /// Try to parse from device byte.
    pub const fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x1F => Some(Self::Stm32F051),
            0x2B => Some(Self::Stm32G071),
            0x35 => Some(Self::Stm32F3),
            _ => None,
        }
    }

    /// Get the page size for this MCU in bytes.
    pub const fn page_size(self) -> usize {
        match self {
            Self::Stm32F051 => 1024,
            Self::Stm32G071 | Self::Stm32F3 => 2048,
        }
    }

    /// Get the EEPROM/config base address for this MCU (protocol address).
    pub const fn eeprom_address(self) -> u16 {
        match self {
            Self::Stm32F051 => 0x7C00,
            Self::Stm32G071 => 0x7E00,
            Self::Stm32F3 => 0xF800,
        }
    }

    /// Check if addresses need to be divided by 4 (G071 quirk).
    pub const fn divides_address(self) -> bool {
        matches!(self, Self::Stm32G071)
    }

    /// Adjust address for this MCU type (divide by 4 for G071).
    pub const fn adjust_address(self, addr: u16) -> u16 {
        if self.divides_address() {
            addr / 4
        } else {
            addr
        }
    }

    /// Get the flash size in bytes.
    pub const fn flash_size(self) -> usize {
        match self {
            Self::Stm32F051 => 32 * 1024,
            Self::Stm32G071 => 128 * 1024,
            Self::Stm32F3 => 64 * 1024,
        }
    }
}

impl core::fmt::Display for FourWayMcuType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Stm32F051 => write!(f, "STM32F051 (32KB, 1KB pages)"),
            Self::Stm32G071 => write!(f, "STM32G071 (128KB, 2KB pages, addr/4)"),
            Self::Stm32F3 => write!(f, "STM32F3xx (64KB, 2KB pages)"),
        }
    }
}

// ============================================================================
// Packet Builders
// ============================================================================

/// Build a simple Four-Way command (no data payload).
///
/// Format: `[CMD] [PARAM] [ADDR_HI] [ADDR_LO] [CRC_LO] [CRC_HI]`
pub fn build_fourway_command(cmd: FourWayCommand, param: u8, address: u16) -> [u8; 6] {
    let mut packet = [0u8; 6];
    packet[0] = cmd.as_byte();
    packet[1] = param;
    packet[2] = (address >> 8) as u8;
    packet[3] = address as u8;
    let crc = crc16_ibm_bytes(&packet[0..4]);
    packet[4] = crc[0]; // CRC LSB
    packet[5] = crc[1]; // CRC MSB
    packet
}

/// Build InterfaceTestAlive command.
#[inline]
pub fn build_test_alive() -> [u8; 6] {
    build_fourway_command(FourWayCommand::InterfaceTestAlive, 0, 0)
}

/// Build ProtocolGetVersion command.
#[inline]
pub fn build_get_version() -> [u8; 6] {
    build_fourway_command(FourWayCommand::ProtocolGetVersion, 0, 0)
}

/// Build InterfaceExit command.
#[inline]
pub fn build_interface_exit() -> [u8; 6] {
    build_fourway_command(FourWayCommand::InterfaceExit, 0, 0)
}

/// Build DeviceInitFlash command (connect to ESC).
///
/// `motor_index` is 0-based motor number.
#[inline]
pub fn build_device_init_flash(motor_index: u8) -> [u8; 6] {
    build_fourway_command(FourWayCommand::DeviceInitFlash, motor_index, 0)
}

/// Build DeviceReset command.
#[inline]
pub fn build_device_reset(motor_index: u8) -> [u8; 6] {
    build_fourway_command(FourWayCommand::DeviceReset, motor_index, 0)
}

/// Build DeviceRead command.
///
/// - `size`: Number of bytes to read (0 = 256)
/// - `address`: Flash address to read from
pub fn build_device_read(size: u8, address: u16) -> [u8; 6] {
    build_fourway_command(FourWayCommand::DeviceRead, size, address)
}

/// Build DeviceReadEeprom command.
///
/// - `size`: Number of bytes to read (0 = 256)
/// - `address`: EEPROM address to read from
pub fn build_device_read_eeprom(size: u8, address: u16) -> [u8; 6] {
    build_fourway_command(FourWayCommand::DeviceReadEeprom, size, address)
}

/// Build DevicePageErase command.
///
/// - `address`: Address of the page to erase
pub fn build_device_page_erase(address: u16) -> [u8; 6] {
    build_fourway_command(FourWayCommand::DevicePageErase, 0, address)
}

/// Build DeviceWrite command with data.
///
/// Format: `[CMD] [SIZE] [ADDR_HI] [ADDR_LO] [DATA...] [CRC_LO] [CRC_HI]`
///
/// - `address`: Flash address to write to
/// - `data`: Data to write (max 256 bytes)
/// - `buffer`: Output buffer (must be at least 6 + data.len() bytes)
///
/// Returns the actual packet length.
pub fn build_device_write(address: u16, data: &[u8], buffer: &mut [u8]) -> usize {
    let size = data.len().min(FOURWAY_MAX_WRITE_SIZE);
    let packet_len = 6 + size;

    assert!(buffer.len() >= packet_len, "buffer too small");

    buffer[0] = FourWayCommand::DeviceWrite.as_byte();
    buffer[1] = size as u8; // 0 means 256
    buffer[2] = (address >> 8) as u8;
    buffer[3] = address as u8;
    buffer[4..4 + size].copy_from_slice(&data[..size]);

    let crc = crc16_ibm_bytes(&buffer[0..4 + size]);
    buffer[4 + size] = crc[0];
    buffer[4 + size + 1] = crc[1];

    packet_len
}

/// Build DeviceWriteEeprom command with data.
///
/// Format: `[CMD] [SIZE] [ADDR_HI] [ADDR_LO] [DATA...] [CRC_LO] [CRC_HI]`
///
/// - `address`: EEPROM address to write to
/// - `data`: Data to write (max 256 bytes)
/// - `buffer`: Output buffer (must be at least 6 + data.len() bytes)
///
/// Returns the actual packet length.
pub fn build_device_write_eeprom(address: u16, data: &[u8], buffer: &mut [u8]) -> usize {
    let size = data.len().min(FOURWAY_MAX_WRITE_SIZE);
    let packet_len = 6 + size;

    assert!(buffer.len() >= packet_len, "buffer too small");

    buffer[0] = FourWayCommand::DeviceWriteEeprom.as_byte();
    buffer[1] = size as u8;
    buffer[2] = (address >> 8) as u8;
    buffer[3] = address as u8;
    buffer[4..4 + size].copy_from_slice(&data[..size]);

    let crc = crc16_ibm_bytes(&buffer[0..4 + size]);
    buffer[4 + size] = crc[0];
    buffer[4 + size + 1] = crc[1];

    packet_len
}

// ============================================================================
// Response Parsing
// ============================================================================

/// Four-Way parsing error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FourWayError {
    /// Response too short
    TooShort,
    /// CRC mismatch
    CrcMismatch,
    /// Invalid ACK value
    InvalidAck { value: u8 },
    /// Command failed (non-OK ACK)
    CommandFailed { ack: FourWayAck },
    /// Unknown MCU type in DeviceInitFlash response
    UnknownMcu { value: u8 },
}

impl core::fmt::Display for FourWayError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort => write!(f, "response too short"),
            Self::CrcMismatch => write!(f, "CRC mismatch"),
            Self::InvalidAck { value } => write!(f, "invalid ACK: 0x{:02X}", value),
            Self::CommandFailed { ack } => write!(f, "command failed: {}", ack),
            Self::UnknownMcu { value } => write!(f, "unknown MCU type: 0x{:02X}", value),
        }
    }
}

/// Parsed Four-Way response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FourWayResponse<'a> {
    /// Command that was echoed back
    pub command: u8,
    /// Parameter/size byte
    pub param: u8,
    /// ACK status
    pub ack: FourWayAck,
    /// Response data (if any)
    pub data: &'a [u8],
    /// Address from response header
    pub address: u16,
}

/// Parse Four-Way response.
///
/// Response format: `[CMD] [PARAM] [ADDR_HI] [ADDR_LO] [DATA...] [ACK] [CRC_LO] [CRC_HI]`
///
/// ACK is at position `len - 3`.
pub fn parse_fourway_response(data: &[u8]) -> Result<FourWayResponse<'_>, FourWayError> {
    if data.len() < FOURWAY_MIN_RESPONSE_SIZE {
        return Err(FourWayError::TooShort);
    }

    // Verify CRC (last 2 bytes)
    let crc_received = u16::from_le_bytes([data[data.len() - 2], data[data.len() - 1]]);
    let crc_calculated = crate::crc::crc16_ibm(&data[..data.len() - 2]);
    if crc_received != crc_calculated {
        return Err(FourWayError::CrcMismatch);
    }

    // ACK is at len-3
    let ack_byte = data[data.len() - 3];
    let ack =
        FourWayAck::from_byte(ack_byte).ok_or(FourWayError::InvalidAck { value: ack_byte })?;

    let command = data[0];
    let param = data[1];

    // Address from bytes 2-3
    let address = u16::from_be_bytes([data[2], data[3]]);

    // Data is between header (4 bytes) and ACK+CRC (3 bytes)
    let data_end = data.len().saturating_sub(3);
    let payload = if data_end > 4 {
        &data[4..data_end]
    } else {
        &[]
    };

    Ok(FourWayResponse {
        command,
        param,
        ack,
        data: payload,
        address,
    })
}

/// Information from DeviceInitFlash response.
#[derive(Debug, Clone)]
pub struct DeviceInitInfo {
    /// Raw MCU type byte
    pub mcu_byte: u8,
    /// Parsed MCU type (if known)
    pub mcu_type: Option<FourWayMcuType>,
}

/// Parse DeviceInitFlash response to get MCU info.
///
/// Response contains device type info.
pub fn parse_init_flash_response(data: &[u8]) -> Result<DeviceInitInfo, FourWayError> {
    let response = parse_fourway_response(data)?;

    if !response.ack.is_ok() {
        return Err(FourWayError::CommandFailed { ack: response.ack });
    }

    // Device type is typically in the param field or first data byte
    // The exact location can vary; we check both
    let mcu_byte = if response.data.is_empty() {
        response.param
    } else {
        response.data[0]
    };

    let mcu_type = FourWayMcuType::from_byte(mcu_byte);

    Ok(DeviceInitInfo { mcu_byte, mcu_type })
}

/// Parse DeviceRead response to extract data.
pub fn parse_read_response(data: &[u8]) -> Result<&[u8], FourWayError> {
    let response = parse_fourway_response(data)?;

    if !response.ack.is_ok() {
        return Err(FourWayError::CommandFailed { ack: response.ack });
    }

    Ok(response.data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;

    #[test]
    fn command_bytes() {
        assert_eq!(FourWayCommand::InterfaceTestAlive.as_byte(), 0x30);
        assert_eq!(FourWayCommand::DeviceInitFlash.as_byte(), 0x37);
        assert_eq!(FourWayCommand::DeviceRead.as_byte(), 0x3A);
        assert_eq!(FourWayCommand::DeviceWrite.as_byte(), 0x3B);
    }

    #[test]
    fn command_from_byte() {
        assert_eq!(
            FourWayCommand::from_byte(0x30),
            Some(FourWayCommand::InterfaceTestAlive)
        );
        assert_eq!(
            FourWayCommand::from_byte(0x37),
            Some(FourWayCommand::DeviceInitFlash)
        );
        assert_eq!(FourWayCommand::from_byte(0x99), None);
    }

    #[test]
    fn ack_from_byte() {
        assert_eq!(FourWayAck::from_byte(0x00), Some(FourWayAck::Ok));
        assert_eq!(FourWayAck::from_byte(0x03), Some(FourWayAck::InvalidCrc));
        assert_eq!(FourWayAck::from_byte(0xFF), None);
    }

    #[test]
    fn mcu_type_properties() {
        assert_eq!(FourWayMcuType::Stm32F051.page_size(), 1024);
        assert_eq!(FourWayMcuType::Stm32G071.page_size(), 2048);
        assert_eq!(FourWayMcuType::Stm32F3.page_size(), 2048);

        assert_eq!(FourWayMcuType::Stm32F051.eeprom_address(), 0x7C00);
        assert_eq!(FourWayMcuType::Stm32G071.eeprom_address(), 0x7E00);
        assert_eq!(FourWayMcuType::Stm32F3.eeprom_address(), 0xF800);

        assert!(!FourWayMcuType::Stm32F051.divides_address());
        assert!(FourWayMcuType::Stm32G071.divides_address());
        assert!(!FourWayMcuType::Stm32F3.divides_address());
    }

    #[test]
    fn address_adjustment() {
        let addr = 0x7E00u16;
        assert_eq!(FourWayMcuType::Stm32G071.adjust_address(addr), 0x1F80);
        assert_eq!(FourWayMcuType::Stm32F051.adjust_address(addr), 0x7E00);
    }

    #[test]
    fn build_init_flash_packet() {
        let packet = build_device_init_flash(0);
        assert_eq!(packet[0], 0x37); // CMD
        assert_eq!(packet[1], 0x00); // Motor index
        assert_eq!(packet[2], 0x00); // Addr high
        assert_eq!(packet[3], 0x00); // Addr low
        // CRC is calculated
    }

    #[test]
    fn build_read_packet() {
        let packet = build_device_read(48, 0x7C00);
        assert_eq!(packet[0], 0x3A); // CMD = DeviceRead
        assert_eq!(packet[1], 48); // Size
        assert_eq!(packet[2], 0x7C); // Addr high
        assert_eq!(packet[3], 0x00); // Addr low
    }

    #[test]
    fn build_write_packet() {
        let mut buffer = [0u8; 64];
        let data = [0x01, 0x02, 0x03, 0x04];
        let len = build_device_write(0x7C00, &data, &mut buffer);

        assert_eq!(len, 10); // 6 + 4 data bytes
        assert_eq!(buffer[0], 0x3B); // CMD = DeviceWrite
        assert_eq!(buffer[1], 4); // Size
        assert_eq!(buffer[2], 0x7C); // Addr high
        assert_eq!(buffer[3], 0x00); // Addr low
        assert_eq!(&buffer[4..8], &data);
    }

    #[test]
    fn parse_response_valid() {
        // Simulated response: CMD=0x3A, PARAM=48, ADDR=0x7C00, DATA=[], ACK=0x00, CRC
        let mut response = vec![0x3A, 48, 0x7C, 0x00, 0x00]; // CMD, PARAM, ADDR, ACK
        let crc = crate::crc::crc16_ibm(&response);
        response.push(crc as u8);
        response.push((crc >> 8) as u8);

        let parsed = parse_fourway_response(&response).unwrap();
        assert_eq!(parsed.command, 0x3A);
        assert_eq!(parsed.param, 48);
        assert_eq!(parsed.ack, FourWayAck::Ok);
        assert_eq!(parsed.address, 0x7C00);
    }

    #[test]
    fn parse_response_with_data() {
        // Response with 4 bytes of data
        let mut response = vec![0x3A, 4, 0x7C, 0x00, 0xAA, 0xBB, 0xCC, 0xDD, 0x00]; // + ACK
        let crc = crate::crc::crc16_ibm(&response);
        response.push(crc as u8);
        response.push((crc >> 8) as u8);

        let parsed = parse_fourway_response(&response).unwrap();
        assert_eq!(parsed.data, &[0xAA, 0xBB, 0xCC, 0xDD]);
        assert_eq!(parsed.ack, FourWayAck::Ok);
    }

    #[test]
    fn parse_response_crc_error() {
        let response = vec![0x3A, 48, 0x7C, 0x00, 0x00, 0xFF, 0xFF]; // Bad CRC
        assert_eq!(
            parse_fourway_response(&response),
            Err(FourWayError::CrcMismatch)
        );
    }

    #[test]
    fn parse_response_too_short() {
        let response = vec![0x3A, 48, 0x7C, 0x00, 0x00]; // Missing CRC
        assert_eq!(
            parse_fourway_response(&response),
            Err(FourWayError::TooShort)
        );
    }
}
