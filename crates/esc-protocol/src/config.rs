//! Config block handling (encryption-aware).
//!
//! The 256-byte config block is encrypted. Each read yields different
//! bytes due to internal counters. Without reverse-engineering the cipher:
//! - Read + store encrypted config
//! - Write back the same encrypted bytes to preserve settings
//!
//! For pure flashing (BlueJay/AM32 firmware), encryption doesn't matter.
//! For per-parameter editing, the cipher must be understood.
//!
//! # Memory Layout
//!
//! Standard addresses (AM32):
//! - `0x7C00`: Main configuration block (256 bytes)
//! - `0xEB00`: Timestamp / extra data (16 bytes)
//! - `0xF7AC`: Device ID / signature (16 bytes)

use crate::crc::crc16_ibm;

/// Size of the main configuration block in bytes.
pub const CONFIG_BLOCK_SIZE: usize = 256;

/// Size of the timestamp block in bytes.
pub const TIMESTAMP_BLOCK_SIZE: usize = 16;

/// Size of the device ID block in bytes.
pub const DEVICE_ID_BLOCK_SIZE: usize = 16;

/// Timestamp block address.
pub const TIMESTAMP_ADDRESS: u16 = 0xEB00;

/// Device ID block address.
pub const DEVICE_ID_ADDRESS: u16 = 0xF7AC;

/// Configuration block (256 bytes, encrypted).
///
/// This is an opaque wrapper around the raw encrypted config data.
/// The contents cannot be interpreted without knowing the encryption key.
#[derive(Clone)]
pub struct ConfigBlock {
    /// Raw encrypted data.
    data: [u8; CONFIG_BLOCK_SIZE],
}

impl ConfigBlock {
    /// Create a new config block from raw bytes.
    ///
    /// # Panics
    /// Panics if data length is not exactly 256 bytes.
    pub fn from_bytes(data: &[u8]) -> Self {
        assert_eq!(
            data.len(),
            CONFIG_BLOCK_SIZE,
            "config block must be 256 bytes"
        );
        let mut block = Self {
            data: [0u8; CONFIG_BLOCK_SIZE],
        };
        block.data.copy_from_slice(data);
        block
    }

    /// Create a config block from a fixed-size array.
    #[inline]
    pub const fn from_array(data: [u8; CONFIG_BLOCK_SIZE]) -> Self {
        Self { data }
    }

    /// Get the raw bytes.
    #[inline]
    pub const fn as_bytes(&self) -> &[u8; CONFIG_BLOCK_SIZE] {
        &self.data
    }

    /// Get mutable reference to raw bytes.
    #[inline]
    pub const fn as_bytes_mut(&mut self) -> &mut [u8; CONFIG_BLOCK_SIZE] {
        &mut self.data
    }

    /// Compute CRC-16 of the config block.
    ///
    /// This is used when writing the config back to the ESC.
    #[inline]
    pub fn crc(&self) -> u16 {
        crc16_ibm(&self.data)
    }

    /// Get CRC bytes in wire order (LSB first).
    #[inline]
    pub fn crc_bytes(&self) -> [u8; 2] {
        let crc = self.crc();
        [crc as u8, (crc >> 8) as u8]
    }
}

impl Default for ConfigBlock {
    fn default() -> Self {
        Self {
            data: [0xFF; CONFIG_BLOCK_SIZE],
        }
    }
}

impl core::fmt::Debug for ConfigBlock {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Don't dump all 256 bytes, just show first/last few
        write!(
            f,
            "ConfigBlock {{ {:02X} {:02X} {:02X} {:02X} ... {:02X} {:02X} {:02X} {:02X} }}",
            self.data[0],
            self.data[1],
            self.data[2],
            self.data[3],
            self.data[252],
            self.data[253],
            self.data[254],
            self.data[255]
        )
    }
}

/// Timestamp/metadata block (16 bytes).
#[derive(Clone, Debug)]
pub struct TimestampBlock {
    data: [u8; TIMESTAMP_BLOCK_SIZE],
}

impl TimestampBlock {
    /// Create from raw bytes.
    pub fn from_bytes(data: &[u8]) -> Self {
        assert_eq!(data.len(), TIMESTAMP_BLOCK_SIZE);
        let mut block = Self {
            data: [0u8; TIMESTAMP_BLOCK_SIZE],
        };
        block.data.copy_from_slice(data);
        block
    }

    /// Get raw bytes.
    #[inline]
    pub const fn as_bytes(&self) -> &[u8; TIMESTAMP_BLOCK_SIZE] {
        &self.data
    }
}

impl Default for TimestampBlock {
    fn default() -> Self {
        Self {
            data: [0xFF; TIMESTAMP_BLOCK_SIZE],
        }
    }
}

/// Device ID/signature block (16 bytes).
#[derive(Clone, Debug)]
pub struct DeviceIdBlock {
    data: [u8; DEVICE_ID_BLOCK_SIZE],
}

impl DeviceIdBlock {
    /// Create from raw bytes.
    pub fn from_bytes(data: &[u8]) -> Self {
        assert_eq!(data.len(), DEVICE_ID_BLOCK_SIZE);
        let mut block = Self {
            data: [0u8; DEVICE_ID_BLOCK_SIZE],
        };
        block.data.copy_from_slice(data);
        block
    }

    /// Get raw bytes.
    #[inline]
    pub const fn as_bytes(&self) -> &[u8; DEVICE_ID_BLOCK_SIZE] {
        &self.data
    }
}

impl Default for DeviceIdBlock {
    fn default() -> Self {
        Self {
            data: [0xFF; DEVICE_ID_BLOCK_SIZE],
        }
    }
}

/// Parse a read response into config block + validate CRC.
///
/// # Arguments
/// * `response` - Raw response bytes (256 data + 2 CRC + 1 ACK = 259 bytes)
///
/// # Returns
/// `Some(ConfigBlock)` if valid, `None` if CRC check fails or response malformed.
pub fn parse_config_response(response: &[u8]) -> Option<ConfigBlock> {
    // Expected: 256 data + 2 CRC + 1 ACK
    if response.len() < CONFIG_BLOCK_SIZE + 3 {
        return None;
    }

    // Extract parts
    let data = &response[..CONFIG_BLOCK_SIZE];
    let crc_received =
        u16::from_le_bytes([response[CONFIG_BLOCK_SIZE], response[CONFIG_BLOCK_SIZE + 1]]);
    let ack = response[CONFIG_BLOCK_SIZE + 2];

    // Verify ACK
    if ack != 0x30 {
        return None;
    }

    // Verify CRC
    let crc_computed = crc16_ibm(data);
    if crc_received != crc_computed {
        return None;
    }

    Some(ConfigBlock::from_bytes(data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::format;

    #[test]
    fn config_block_size() {
        assert_eq!(CONFIG_BLOCK_SIZE, 256);
    }

    #[test]
    fn config_block_from_bytes() {
        let data = [0xAB; 256];
        let block = ConfigBlock::from_bytes(&data);
        assert_eq!(block.as_bytes()[0], 0xAB);
        assert_eq!(block.as_bytes()[255], 0xAB);
    }

    #[test]
    fn config_block_crc() {
        let block = ConfigBlock::from_array([0x00; 256]);
        let crc = block.crc();
        // CRC of all zeros
        assert_eq!(crc, 0x0000);

        let bytes = block.crc_bytes();
        assert_eq!(bytes, [0x00, 0x00]);
    }

    #[test]
    fn config_block_debug() {
        let block = ConfigBlock::default();
        let debug = format!("{:?}", block);
        assert!(debug.contains("ConfigBlock"));
        assert!(debug.contains("FF"));
    }

    #[test]
    fn parse_config_response_valid() {
        // Create test response: 256 bytes data + 2 CRC + 1 ACK
        let mut response = [0u8; 259];
        response[..256].fill(0x42);

        // Compute CRC
        let crc = crc16_ibm(&response[..256]);
        response[256] = crc as u8;
        response[257] = (crc >> 8) as u8;
        response[258] = 0x30; // ACK

        let block = parse_config_response(&response).unwrap();
        assert_eq!(block.as_bytes()[0], 0x42);
    }

    #[test]
    fn parse_config_response_bad_crc() {
        let mut response = [0u8; 259];
        response[..256].fill(0x42);
        response[256] = 0xFF; // Wrong CRC
        response[257] = 0xFF;
        response[258] = 0x30;

        assert!(parse_config_response(&response).is_none());
    }

    #[test]
    fn parse_config_response_bad_ack() {
        let mut response = [0u8; 259];
        response[..256].fill(0x42);
        let crc = crc16_ibm(&response[..256]);
        response[256] = crc as u8;
        response[257] = (crc >> 8) as u8;
        response[258] = 0x00; // Wrong ACK

        assert!(parse_config_response(&response).is_none());
    }

    #[test]
    fn parse_config_response_too_short() {
        let response = [0u8; 100];
        assert!(parse_config_response(&response).is_none());
    }

    #[test]
    fn timestamp_block() {
        let data = [0x12; 16];
        let block = TimestampBlock::from_bytes(&data);
        assert_eq!(block.as_bytes()[0], 0x12);
    }

    #[test]
    fn device_id_block() {
        let data = [0x34; 16];
        let block = DeviceIdBlock::from_bytes(&data);
        assert_eq!(block.as_bytes()[0], 0x34);
    }
}
