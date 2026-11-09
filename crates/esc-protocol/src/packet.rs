//! Frames of the AM32 bootloader serial protocol.
//!
//! Every command frame ends with a CRC-16/IBM of the preceding bytes, LSB first.
//! The bootloader frames by line idle time, so each frame built here must go out
//! back to back, and two frames need a gap between them (see `docs/REVAMP.md`,
//! "How the bootloader actually behaves").
//!
//! | Frame | Bytes | Reply |
//! |---|---|---|
//! | Connect | 12×`00` `0D` `"BLHeli"` CRC | 9-byte device info |
//! | Set Address | `FF 00 HI LO` CRC | `30` |
//! | Set Buffer | `FE 00 01 00` (256) or `FE 00 00 N` CRC | none |
//! | Payload | data, CRC | `30`, or `C2` (bad CRC; still waiting for a payload) |
//! | Prog Flash | `01 01` CRC | `30`, or `C1` |
//! | Read | `03 N` CRC (`N = 0`: 256) | data, CRC, `30` |
//! | Run | `00 00 00 00` | `C1`, then the app starts |

use crate::crc::{crc16_ibm, crc16_ibm_bytes};

// ============================================================================
// Protocol Constants
// ============================================================================

/// Boot unlock preamble length (12 bytes of 0x00 for Direct protocol).
pub const BOOT_PREAMBLE_LEN: usize = 12;

/// Direct protocol marker byte (0x0D) before "BLHeli" signature.
pub const DIRECT_MARKER: u8 = 0x0D;

/// BLHeli signature string.
pub const BLHELI_SIGNATURE: &[u8; 6] = b"BLHeli";

/// Reply to a command that succeeded.
pub const ACK: u8 = 0x30;

/// Reply to an unknown command, a refused address, a failed flash write, a
/// keepalive and a Run.
pub const BAD_COMMAND: u8 = 0xC1;

/// Reply to a frame whose CRC does not match.
pub const BAD_CRC: u8 = 0xC2;

/// Connect response prefix ("471").
pub const CONNECT_RESPONSE_PREFIX: &[u8; 3] = b"471";

/// Length of the connect reply.
pub const CONNECT_RESPONSE_LEN: usize = 9;

/// Baud rate for BLHeli protocol.
pub const BAUD_RATE: u32 = 19200;

/// Largest buffer the bootloader accepts for a write or read.
pub const MAX_BLOCK: usize = 256;

// ----------------------------------------------------------------------------
// Magic Set Address values (bootloader protocol version 2+, devinfo: 3+)
// ----------------------------------------------------------------------------

/// Set Address value that selects the EEPROM base.
pub const ADDR_MAGIC_EEPROM: u16 = 0x20;
/// Set Address value that selects the firmware file name (32 bytes).
pub const ADDR_MAGIC_FILE_NAME: u16 = 0x21;
/// Set Address value that continues after the last read.
pub const ADDR_MAGIC_CONTINUE: u16 = 0x22;
/// Set Address value that selects the devinfo structure (protocol version 3+).
pub const ADDR_MAGIC_DEVINFO: u16 = 0x23;

/// Set Address values below this (other than the magics) are refused.
pub const MIN_PLAIN_ADDRESS: u16 = 1024;

// ============================================================================
// Frame Builders
// ============================================================================

const fn with_crc4(a: u8, b: u8) -> [u8; 4] {
    let crc = crc16_ibm(&[a, b]);
    [a, b, crc as u8, (crc >> 8) as u8]
}

const fn with_crc6(a: u8, b: u8, c: u8, d: u8) -> [u8; 6] {
    let crc = crc16_ibm(&[a, b, c, d]);
    [a, b, c, d, crc as u8, (crc >> 8) as u8]
}

/// Build connect packet (Direct protocol).
///
/// Format: 12x 0x00 (boot unlock) + 0x0D + "BLHeli" + CRC of "BLHeli"
#[inline]
pub fn build_connect_packet() -> [u8; 21] {
    let mut packet = [0u8; 21];
    packet[BOOT_PREAMBLE_LEN] = DIRECT_MARKER;
    packet[13..19].copy_from_slice(BLHELI_SIGNATURE);
    let crc = crc16_ibm_bytes(BLHELI_SIGNATURE);
    packet[19] = crc[0]; // 0xF4
    packet[20] = crc[1]; // 0x7D (the bootloader keys on this byte at index 20)
    packet
}

/// Build Set Address: `[FF 00 HI LO]` + CRC.
///
/// `address` is in protocol units: shifted right by the device's address shift on
/// 128 KiB parts. See [`crate::layout::Layout`].
#[inline]
pub const fn build_set_address_packet(address: u16) -> [u8; 6] {
    with_crc6(0xFF, 0x00, (address >> 8) as u8, address as u8)
}

/// Build Set Buffer for a payload of `size` bytes (1..=256).
///
/// `FE 00 01 00` means 256; `FE 00 00 N` means N. `FE 00 00 00` would announce an
/// empty buffer, which is why 256 must use the long form. No reply on success.
#[inline]
pub const fn build_set_buffer_packet(size: u16) -> Option<[u8; 6]> {
    match size {
        1..=255 => Some(with_crc6(0xFE, 0x00, 0x00, size as u8)),
        256 => Some(with_crc6(0xFE, 0x00, 0x01, 0x00)),
        _ => None,
    }
}

/// Build a payload frame: `data` followed by its CRC.
///
/// Writes into `out` and returns the frame length, or `None` if `data` is empty,
/// longer than 256 bytes, or `out` is too small.
pub fn build_payload(data: &[u8], out: &mut [u8]) -> Option<usize> {
    let n = data.len();
    if n == 0 || n > MAX_BLOCK || out.len() < n + 2 {
        return None;
    }
    out[..n].copy_from_slice(data);
    out[n..n + 2].copy_from_slice(&crc16_ibm_bytes(data));
    Some(n + 2)
}

/// Build Prog Flash: write the buffer at the current address. `[01 01]` + CRC.
#[inline]
pub const fn build_prog_flash_packet() -> [u8; 4] {
    with_crc4(0x01, 0x01)
}

/// Build Read for `len` bytes (1..=256) at the current address.
#[inline]
pub const fn build_read_packet(len: u16) -> Option<[u8; 4]> {
    match len {
        1..=255 => Some(with_crc4(0x03, len as u8)),
        256 => Some(with_crc4(0x03, 0x00)),
        _ => None,
    }
}

/// Build Run: leave the bootloader and start the application.
///
/// `[00 00]` + CRC, and the CRC of `00 00` is `0000`. The bootloader still replies
/// [`BAD_COMMAND`], then jumps if EEPROM byte 0 is `0x01` and the app looks valid.
#[inline]
pub const fn build_run_packet() -> [u8; 4] {
    with_crc4(0x00, 0x00)
}

// ============================================================================
// Reply Parsing
// ============================================================================

/// Parsed connect reply: `'4' '7' '1' PIN FLASH b5 b6 PROTO 0x30`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectResponse {
    /// `PIN | FLASH << 8`, the historical "device id".
    pub device_id: u16,
    /// Signal pin code: `port << 4 | pin` on AM32.
    pub pin_code: u8,
    /// Flash size code: `0x1F` 32K, `0x35` 64K, `0x2B` 128K on AM32.
    pub flash_code: u8,
    /// Bootloader protocol version (AM32: byte 7).
    pub protocol_version: u8,
    /// Raw reply bytes.
    pub raw: [u8; CONNECT_RESPONSE_LEN],
}

/// Parse the 9-byte connect reply. `None` if the prefix or trailing ACK is wrong.
pub fn parse_connect_response(data: &[u8]) -> Option<ConnectResponse> {
    let raw: [u8; CONNECT_RESPONSE_LEN] = data.get(..CONNECT_RESPONSE_LEN)?.try_into().ok()?;
    if &raw[0..3] != CONNECT_RESPONSE_PREFIX || raw[8] != ACK {
        return None;
    }
    Some(ConnectResponse {
        device_id: u16::from_le_bytes([raw[3], raw[4]]),
        pin_code: raw[3],
        flash_code: raw[4],
        protocol_version: raw[7],
        raw,
    })
}

/// Why a read reply was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadReplyError {
    /// Fewer than `len + 3` bytes.
    TooShort,
    /// The trailing byte is not [`ACK`]; holds the byte.
    NotAcked(u8),
    /// The data does not match its CRC.
    BadCrc,
}

/// Check a read reply (`data`, CRC LSB first, `0x30`) and return the data.
pub fn parse_read_reply(reply: &[u8], len: usize) -> Result<&[u8], ReadReplyError> {
    if reply.len() < len + 3 {
        return Err(ReadReplyError::TooShort);
    }
    if reply[len + 2] != ACK {
        return Err(ReadReplyError::NotAcked(reply[len + 2]));
    }
    let data = &reply[..len];
    if crc16_ibm_bytes(data) != [reply[len], reply[len + 1]] {
        return Err(ReadReplyError::BadCrc);
    }
    Ok(data)
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
        assert!(packet[..12].iter().all(|&b| b == 0));
        assert_eq!(packet[12], 0x0D);
        assert_eq!(&packet[13..19], b"BLHeli");
        assert_eq!(&packet[19..21], &[0xF4, 0x7D]);
    }

    #[test]
    fn set_address_packet_7c00() {
        assert_eq!(
            build_set_address_packet(0x7C00),
            [0xFF, 0x00, 0x7C, 0x00, 0x10, 0xD4]
        );
    }

    #[test]
    fn set_buffer_encodes_256_with_byte_2() {
        // P4: FE 00 00 00 would announce an empty buffer.
        assert_eq!(
            build_set_buffer_packet(256),
            Some([0xFE, 0x00, 0x01, 0x00, 0x30, 0x78])
        );
        let p = build_set_buffer_packet(192).unwrap();
        assert_eq!(&p[..4], &[0xFE, 0x00, 0x00, 0xC0]);
        assert_eq!(crc16_ibm_bytes(&p[..4]), [p[4], p[5]]);
        assert_eq!(build_set_buffer_packet(0), None);
        assert_eq!(build_set_buffer_packet(257), None);
    }

    #[test]
    fn read_packet_lengths() {
        assert_eq!(build_read_packet(256), Some([0x03, 0x00, 0x00, 0xF0]));
        assert_eq!(build_read_packet(16), Some([0x03, 0x10, 0x01, 0x3C]));
        assert_eq!(build_read_packet(0), None);
        assert_eq!(build_read_packet(300), None);
    }

    #[test]
    fn prog_flash_and_run_packets() {
        assert_eq!(build_prog_flash_packet(), [0x01, 0x01, 0xC0, 0x50]);
        assert_eq!(build_run_packet(), [0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn payload_appends_crc() {
        let mut out = [0u8; 10];
        let n = build_payload(&[1, 2, 3, 4], &mut out).unwrap();
        assert_eq!(n, 6);
        assert_eq!(&out[4..6], &crc16_ibm_bytes(&[1, 2, 3, 4]));
        assert_eq!(build_payload(&[], &mut out), None);
        assert_eq!(build_payload(&[0; 9], &mut out), None);
    }

    #[test]
    fn parse_am32_connect_response() {
        // F051, pin PA6, protocol 3
        let r = parse_connect_response(b"471\x06\x1F\x06\x06\x03\x30").unwrap();
        assert_eq!(r.device_id, 0x1F06);
        assert_eq!(
            (r.pin_code, r.flash_code, r.protocol_version),
            (0x06, 0x1F, 3)
        );
    }

    #[test]
    fn parse_invalid_connect_responses() {
        assert!(parse_connect_response(b"472\x06\x1F\x06\x06\x03\x30").is_none());
        assert!(parse_connect_response(b"471\x06\x1F\x06\x06\x03\x31").is_none());
        assert!(parse_connect_response(b"471\x06\x1F").is_none());
    }

    #[test]
    fn read_reply_checks_crc_and_ack() {
        let data = [9u8, 8, 7];
        let crc = crc16_ibm_bytes(&data);
        let good = [9, 8, 7, crc[0], crc[1], ACK];
        assert_eq!(parse_read_reply(&good, 3), Ok(&data[..]));

        let mut bad_crc = good;
        bad_crc[1] ^= 0xFF;
        assert_eq!(parse_read_reply(&bad_crc, 3), Err(ReadReplyError::BadCrc));

        let mut nack = good;
        nack[5] = BAD_COMMAND;
        assert_eq!(
            parse_read_reply(&nack, 3),
            Err(ReadReplyError::NotAcked(BAD_COMMAND))
        );
        assert_eq!(
            parse_read_reply(&good[..5], 3),
            Err(ReadReplyError::TooShort)
        );
    }
}
