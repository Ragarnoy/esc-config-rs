//! Packet builders for BLHeli protocol.
//!
//! This module provides functions to build protocol packets:
//! - Connect handshake
//! - Keep-alive
//! - Disconnect
//! - Set address
//! - Read block
//! - Write block
//! - Commit flash
//!
//! All packets are CRC-16/IBM protected with CRC sent LSB first.

use crate::crc::crc16_ibm_bytes;

// ============================================================================
// Protocol Constants
// ============================================================================

/// Boot unlock preamble length (12 bytes of 0x00 for Direct protocol).
pub const BOOT_PREAMBLE_LEN: usize = 12;

/// Direct protocol marker byte (0x0D) before "BLHeli" signature.
pub const DIRECT_MARKER: u8 = 0x0D;

/// BLHeli signature string.
pub const BLHELI_SIGNATURE: &[u8; 6] = b"BLHeli";

/// ACK byte returned by ESC on success.
pub const ACK: u8 = 0x30;

/// Keep-alive ACK byte.
pub const KEEPALIVE_ACK: u8 = 0xC1;

/// Connect response prefix ("471").
pub const CONNECT_RESPONSE_PREFIX: &[u8; 3] = b"471";

/// Baud rate for BLHeli protocol.
pub const BAUD_RATE: u32 = 19200;

// ----------------------------------------------------------------------------
// Opcodes
// ----------------------------------------------------------------------------

/// Keep-alive opcode.
pub const OP_KEEPALIVE: [u8; 2] = [0xFD, 0x00];

/// Disconnect opcode.
pub const OP_DISCONNECT: [u8; 2] = [0x00, 0x01];

/// Set address opcode.
pub const OP_SET_ADDRESS: [u8; 2] = [0xFF, 0x00];

/// Read opcode (high byte is length: 0x00 = 256 bytes).
pub const OP_READ_256: [u8; 2] = [0x03, 0x00];

/// Read opcode for 16 bytes.
pub const OP_READ_16: [u8; 2] = [0x03, 0x10];

/// Read opcode for 32 bytes (ESC file name).
pub const OP_READ_32: [u8; 2] = [0x03, 0x20];

/// Read opcode for 48 bytes (used by Arduino before write).
pub const OP_READ_48: [u8; 2] = [0x03, 0x30];

/// Read opcode for 128 bytes (used by Arduino before write).
pub const OP_READ_128: [u8; 2] = [0x03, 0x80];

/// Write header opcode (256 bytes) - long format with explicit page count.
pub const OP_WRITE_256: [u8; 4] = [0xFE, 0x00, 0x01, 0x00];

/// Write header opcode (256 bytes) - short format used by AM32.
pub const OP_WRITE_PAGE: [u8; 2] = [0xFE, 0x00];

/// Commit flash opcode.
pub const OP_COMMIT: [u8; 2] = [0x01, 0x01];

/// Erase page opcode (erases page at current address).
pub const OP_ERASE_PAGE: [u8; 2] = [0x02, 0x00];

// ----------------------------------------------------------------------------
// Known CRC values (compile-time verified)
// ----------------------------------------------------------------------------

/// CRC of "BLHeli" signature.
pub const CRC_BLHELI: u16 = 0x7DF4;

/// CRC of keep-alive opcode.
pub const CRC_KEEPALIVE: u16 = 0x9040;

/// CRC of disconnect opcode.
pub const CRC_DISCONNECT: u16 = 0xC0C1;

/// CRC of write header (256 bytes) - long format.
pub const CRC_WRITE_256: u16 = 0x7830;

/// CRC of write header - short format (FE 00).
pub const CRC_WRITE_PAGE: u16 = 0x6040;

/// CRC of commit flash opcode.
pub const CRC_COMMIT: u16 = 0x50C0;

/// CRC of erase page opcode.
pub const CRC_ERASE_PAGE: u16 = 0x6001;

/// CRC of read 256 bytes opcode.
pub const CRC_READ_256: u16 = 0xF000;

/// CRC of read 16 bytes opcode.
pub const CRC_READ_16: u16 = 0x3C01;

/// CRC of read 32 bytes opcode.
pub const CRC_READ_32: u16 = 0x2801;

/// CRC of read 48 bytes opcode.
pub const CRC_READ_48: u16 = 0xE400;

/// CRC of read 128 bytes opcode.
pub const CRC_READ_128: u16 = 0x5001;

// ----------------------------------------------------------------------------
// Standard addresses
// ----------------------------------------------------------------------------

/// Main config block address.
pub const ADDR_CONFIG: u16 = 0x7C00;

/// Timestamp / extra data address.
pub const ADDR_TIMESTAMP: u16 = 0xEB00;

/// Device ID / signature address.
pub const ADDR_DEVICE_ID: u16 = 0xF7AC;

// ============================================================================
// Packet Builders
// ============================================================================

/// Build connect packet (Direct protocol).
///
/// Format: 12x 0x00 (boot unlock) + 0x0D + "BLHeli" + CRC
///
/// Returns a 21-byte array.
#[inline]
pub fn build_connect_packet() -> [u8; 21] {
    let mut packet = [0u8; 21];
    // First 12 bytes are already 0x00 (boot unlock)
    packet[12] = DIRECT_MARKER; // 0x0D
    packet[13..19].copy_from_slice(BLHELI_SIGNATURE);
    let crc = crc16_ibm_bytes(BLHELI_SIGNATURE);
    packet[19] = crc[0]; // LSB = 0xF4
    packet[20] = crc[1]; // MSB = 0x7D
    packet
}

/// Build keep-alive packet.
///
/// Format: [FD 00] + CRC = [FD 00 40 90]
#[inline]
pub const fn build_keepalive_packet() -> [u8; 4] {
    [
        OP_KEEPALIVE[0],
        OP_KEEPALIVE[1],
        CRC_KEEPALIVE as u8,        // LSB = 0x40
        (CRC_KEEPALIVE >> 8) as u8, // MSB = 0x90
    ]
}

/// Build disconnect packet.
///
/// Format: [00 01] + CRC = [00 01 C1 C0]
#[inline]
pub const fn build_disconnect_packet() -> [u8; 4] {
    [
        OP_DISCONNECT[0],
        OP_DISCONNECT[1],
        CRC_DISCONNECT as u8,        // LSB = 0xC1
        (CRC_DISCONNECT >> 8) as u8, // MSB = 0xC0
    ]
}

/// Build set-address packet.
///
/// Format: [FF 00 addr_hi addr_lo] + CRC
#[inline]
pub fn build_set_address_packet(address: u16) -> [u8; 6] {
    let mut packet = [0u8; 6];
    packet[0] = OP_SET_ADDRESS[0]; // 0xFF
    packet[1] = OP_SET_ADDRESS[1]; // 0x00
    packet[2] = (address >> 8) as u8; // addr MSB (high byte first)
    packet[3] = address as u8; // addr LSB
    let crc = crc16_ibm_bytes(&packet[0..4]);
    packet[4] = crc[0];
    packet[5] = crc[1];
    packet
}

/// Build read packet for 256 bytes.
///
/// Format: [03 00] + CRC = [03 00 00 F0]
#[inline]
pub const fn build_read_256_packet() -> [u8; 4] {
    [
        OP_READ_256[0],
        OP_READ_256[1],
        CRC_READ_256 as u8,        // LSB = 0x00
        (CRC_READ_256 >> 8) as u8, // MSB = 0xF0
    ]
}

/// Build read packet for 16 bytes.
///
/// Format: [03 10] + CRC = [03 10 01 3C]
#[inline]
pub const fn build_read_16_packet() -> [u8; 4] {
    [
        OP_READ_16[0],
        OP_READ_16[1],
        CRC_READ_16 as u8,        // LSB = 0x01
        (CRC_READ_16 >> 8) as u8, // MSB = 0x3C
    ]
}

/// Build read packet for 32 bytes.
///
/// Format: [03 20] + CRC = [03 20 01 28]
/// Used to read ESC file name (stored 32 bytes before eeprom offset).
#[inline]
pub const fn build_read_32_packet() -> [u8; 4] {
    [
        OP_READ_32[0],
        OP_READ_32[1],
        CRC_READ_32 as u8,        // LSB = 0x01
        (CRC_READ_32 >> 8) as u8, // MSB = 0x28
    ]
}

/// Build read packet for 48 bytes.
///
/// Format: [03 30] + CRC = [03 30 00 E4]
/// Used by Arduino before write operations.
#[inline]
pub const fn build_read_48_packet() -> [u8; 4] {
    [
        OP_READ_48[0],
        OP_READ_48[1],
        CRC_READ_48 as u8,        // LSB = 0x00
        (CRC_READ_48 >> 8) as u8, // MSB = 0xE4
    ]
}

/// Build read packet for 128 bytes.
///
/// Format: [03 80] + CRC = [03 80 01 50]
/// Used by Arduino before write operations.
#[inline]
pub const fn build_read_128_packet() -> [u8; 4] {
    [
        OP_READ_128[0],
        OP_READ_128[1],
        CRC_READ_128 as u8,        // LSB = 0x01
        (CRC_READ_128 >> 8) as u8, // MSB = 0x50
    ]
}

/// Build write header packet for 256 bytes (long format).
///
/// Format: [FE 00 01 00] + CRC = [FE 00 01 00 30 78]
///
/// Note: This is the BLHeli_S format with explicit page count.
/// For AM32, use `build_write_page_packet()` instead.
#[inline]
pub const fn build_write_header_packet() -> [u8; 6] {
    [
        OP_WRITE_256[0],
        OP_WRITE_256[1],
        OP_WRITE_256[2],
        OP_WRITE_256[3],
        CRC_WRITE_256 as u8,        // LSB = 0x30
        (CRC_WRITE_256 >> 8) as u8, // MSB = 0x78
    ]
}

/// Build write page packet (short format for AM32).
///
/// Format: [FE 00] + CRC = [FE 00 40 60]
///
/// This is the shorter write command format used by AM32 bootloaders.
/// After sending this header, wait for ACK, then send data + CRC.
#[inline]
pub const fn build_write_page_packet() -> [u8; 4] {
    [
        OP_WRITE_PAGE[0],
        OP_WRITE_PAGE[1],
        CRC_WRITE_PAGE as u8,        // LSB = 0x40
        (CRC_WRITE_PAGE >> 8) as u8, // MSB = 0x60
    ]
}

/// Build commit flash packet.
///
/// Format: [01 01] + CRC = [01 01 C0 50]
#[inline]
pub const fn build_commit_packet() -> [u8; 4] {
    [
        OP_COMMIT[0],
        OP_COMMIT[1],
        CRC_COMMIT as u8,        // LSB = 0xC0
        (CRC_COMMIT >> 8) as u8, // MSB = 0x50
    ]
}

/// Build erase page packet.
///
/// Erases the flash page at the currently set address.
/// Format: [02 00] + CRC = [02 00 01 60]
#[inline]
pub const fn build_erase_page_packet() -> [u8; 4] {
    [
        OP_ERASE_PAGE[0],
        OP_ERASE_PAGE[1],
        CRC_ERASE_PAGE as u8,        // LSB = 0x01
        (CRC_ERASE_PAGE >> 8) as u8, // MSB = 0x60
    ]
}

/// Append CRC to data for write operations.
///
/// Returns the 2-byte CRC in wire order (LSB first).
#[inline]
pub fn compute_data_crc(data: &[u8]) -> [u8; 2] {
    crc16_ibm_bytes(data)
}

// ============================================================================
// Response Parsing
// ============================================================================

/// Parsed connect response from ESC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectResponse {
    /// Device type identifier (e.g., 0x336A).
    pub device_id: u16,
    /// Raw response bytes for debugging.
    pub raw: [u8; 9],
}

/// Parse connect response from ESC.
///
/// Expected format: "471" + device_id (2 bytes) + diagnostics (3 bytes) + ACK
///
/// Returns `None` if the response is malformed.
pub fn parse_connect_response(data: &[u8]) -> Option<ConnectResponse> {
    if data.len() < 9 {
        return None;
    }

    // Check "471" prefix
    if &data[0..3] != CONNECT_RESPONSE_PREFIX {
        return None;
    }

    // Check ACK at end
    if data[8] != ACK {
        return None;
    }

    let device_id = u16::from_le_bytes([data[3], data[4]]);

    let mut raw = [0u8; 9];
    raw.copy_from_slice(&data[0..9]);

    Some(ConnectResponse { device_id, raw })
}

/// Check if response is a simple ACK.
#[inline]
pub const fn is_ack(byte: u8) -> bool {
    byte == ACK
}

/// Check if response is a keep-alive ACK.
#[inline]
pub const fn is_keepalive_ack(byte: u8) -> bool {
    byte == KEEPALIVE_ACK
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_packet_format() {
        let packet = build_connect_packet();

        // First 12 bytes: boot unlock (0x00)
        assert_eq!(&packet[0..12], &[0x00; 12]);

        // Byte 12: Direct marker (0x0D)
        assert_eq!(packet[12], 0x0D);

        // Next 6 bytes: "BLHeli"
        assert_eq!(&packet[13..19], b"BLHeli");

        // Last 2 bytes: CRC (0x7DF4 in wire order = [F4 7D])
        assert_eq!(&packet[19..21], &[0xF4, 0x7D]);
    }

    #[test]
    fn keepalive_packet_exact() {
        assert_eq!(build_keepalive_packet(), [0xFD, 0x00, 0x40, 0x90]);
    }

    #[test]
    fn disconnect_packet_exact() {
        assert_eq!(build_disconnect_packet(), [0x00, 0x01, 0xC1, 0xC0]);
    }

    #[test]
    fn set_address_packet_7c00() {
        let packet = build_set_address_packet(0x7C00);
        // [FF 00 7C 00] + CRC(0xD410) = [FF 00 7C 00 10 D4]
        assert_eq!(packet, [0xFF, 0x00, 0x7C, 0x00, 0x10, 0xD4]);
    }

    #[test]
    fn set_address_packet_eb00() {
        let packet = build_set_address_packet(0xEB00);
        // [FF 00 EB 00] + CRC(0xE47E) = [FF 00 EB 00 7E E4]
        assert_eq!(packet, [0xFF, 0x00, 0xEB, 0x00, 0x7E, 0xE4]);
    }

    #[test]
    fn read_256_packet_exact() {
        assert_eq!(build_read_256_packet(), [0x03, 0x00, 0x00, 0xF0]);
    }

    #[test]
    fn read_16_packet_exact() {
        assert_eq!(build_read_16_packet(), [0x03, 0x10, 0x01, 0x3C]);
    }

    #[test]
    fn write_header_packet_exact() {
        assert_eq!(
            build_write_header_packet(),
            [0xFE, 0x00, 0x01, 0x00, 0x30, 0x78]
        );
    }

    #[test]
    fn write_page_packet_exact() {
        // Short format for AM32
        assert_eq!(build_write_page_packet(), [0xFE, 0x00, 0x40, 0x60]);
    }

    #[test]
    fn commit_packet_exact() {
        assert_eq!(build_commit_packet(), [0x01, 0x01, 0xC0, 0x50]);
    }

    #[test]
    fn erase_page_packet_exact() {
        assert_eq!(build_erase_page_packet(), [0x02, 0x00, 0x01, 0x60]);
    }

    #[test]
    fn parse_valid_connect_response() {
        // "471" + device_id 0x336A + 3 diagnostic bytes + ACK
        let data = [0x34, 0x37, 0x31, 0x6A, 0x33, 0x06, 0x07, 0x04, 0x30];
        let resp = parse_connect_response(&data).unwrap();

        assert_eq!(resp.device_id, 0x336A);
        assert_eq!(resp.raw, data);
    }

    #[test]
    fn parse_invalid_connect_response_wrong_prefix() {
        let data = [0x34, 0x37, 0x32, 0x6A, 0x33, 0x06, 0x07, 0x04, 0x30];
        assert!(parse_connect_response(&data).is_none());
    }

    #[test]
    fn parse_invalid_connect_response_no_ack() {
        let data = [0x34, 0x37, 0x31, 0x6A, 0x33, 0x06, 0x07, 0x04, 0x00];
        assert!(parse_connect_response(&data).is_none());
    }

    #[test]
    fn parse_invalid_connect_response_too_short() {
        let data = [0x34, 0x37, 0x31, 0x6A];
        assert!(parse_connect_response(&data).is_none());
    }

    #[test]
    fn ack_checks() {
        assert!(is_ack(0x30));
        assert!(!is_ack(0x00));
        assert!(is_keepalive_ack(0xC1));
        assert!(!is_keepalive_ack(0x30));
    }
}
