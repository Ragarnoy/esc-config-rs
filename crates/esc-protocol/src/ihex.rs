//! Intel HEX file format parser.
//!
//! Intel HEX is a text format for representing binary data, commonly used
//! for firmware distribution. Each line represents a record with address,
//! type, data, and checksum.
//!
//! # Record Format
//!
//! ```text
//! :LLAAAATT[DD...]CC
//!  ││││││││      ││
//!  ││││││││      └┴─ Checksum (2 hex chars)
//!  ││││││└┴──────── Data bytes (2 hex chars each)
//!  ││││└┴────────── Record type (2 hex chars)
//!  │└┴┴──────────── Address (4 hex chars)
//!  └─────────────── Byte count (2 hex chars)
//! ```
//!
//! # Example
//!
//! ```
//! use esc_protocol::ihex::{parse_record, RecordType};
//!
//! let record = parse_record(":100000000C9434000C9451000C9451000C94510049").unwrap();
//! assert_eq!(record.record_type, RecordType::Data);
//! assert_eq!(record.address, 0x0000);
//! assert_eq!(record.data.len(), 16);
//! ```

use core::fmt;

/// Maximum data bytes per record (standard is 255, but 32 is common).
pub const MAX_RECORD_DATA: usize = 255;

/// Intel HEX record types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordType {
    /// Data record - contains data bytes to write at the specified address.
    Data = 0x00,
    /// End of file record - marks the end of the hex file.
    Eof = 0x01,
    /// Extended segment address - sets bits 4-19 of the segment base address.
    ExtendedSegmentAddress = 0x02,
    /// Start segment address - specifies the CS:IP for 8086 (rarely used).
    StartSegmentAddress = 0x03,
    /// Extended linear address - sets bits 16-31 of the linear base address.
    ExtendedLinearAddress = 0x04,
    /// Start linear address - specifies the EIP for 386+ (entry point).
    StartLinearAddress = 0x05,
}

impl RecordType {
    /// Parse record type from byte value.
    pub const fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x00 => Some(Self::Data),
            0x01 => Some(Self::Eof),
            0x02 => Some(Self::ExtendedSegmentAddress),
            0x03 => Some(Self::StartSegmentAddress),
            0x04 => Some(Self::ExtendedLinearAddress),
            0x05 => Some(Self::StartLinearAddress),
            _ => None,
        }
    }
}

/// A parsed Intel HEX record.
#[derive(Clone, PartialEq, Eq)]
pub struct Record {
    /// 16-bit address offset within the current segment/linear address.
    pub address: u16,
    /// Record type.
    pub record_type: RecordType,
    /// Data bytes (length depends on record type).
    pub data: RecordData,
}

impl fmt::Debug for Record {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Record")
            .field("address", &format_args!("0x{:04X}", self.address))
            .field("record_type", &self.record_type)
            .field("data", &self.data)
            .finish()
    }
}

/// Fixed-capacity buffer for record data.
#[derive(Clone, PartialEq, Eq)]
pub struct RecordData {
    data: [u8; MAX_RECORD_DATA],
    len: usize,
}

impl RecordData {
    /// Create empty record data.
    pub const fn new() -> Self {
        Self {
            data: [0; MAX_RECORD_DATA],
            len: 0,
        }
    }

    /// Get the data as a slice.
    pub fn as_slice(&self) -> &[u8] {
        &self.data[..self.len]
    }

    /// Get the number of data bytes.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Check if empty.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Push a byte. Returns false if full.
    const fn push(&mut self, byte: u8) -> bool {
        if self.len < MAX_RECORD_DATA {
            self.data[self.len] = byte;
            self.len += 1;
            true
        } else {
            false
        }
    }
}

impl Default for RecordData {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for RecordData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[")?;
        for (i, byte) in self.as_slice().iter().enumerate() {
            if i > 0 {
                write!(f, " ")?;
            }
            write!(f, "{byte:02X}")?;
        }
        write!(f, "]")
    }
}

/// Errors that can occur when parsing Intel HEX records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// Line doesn't start with ':'.
    MissingStartCode,
    /// Line is too short to contain a valid record.
    TooShort,
    /// Invalid hexadecimal character.
    InvalidHex,
    /// Checksum doesn't match.
    ChecksumMismatch { expected: u8, actual: u8 },
    /// Unknown record type.
    UnknownRecordType(u8),
    /// Byte count doesn't match data length.
    ByteCountMismatch { expected: usize, actual: usize },
    /// Data too long for buffer.
    DataTooLong,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingStartCode => write!(f, "missing start code ':'"),
            Self::TooShort => write!(f, "record too short"),
            Self::InvalidHex => write!(f, "invalid hexadecimal character"),
            Self::ChecksumMismatch { expected, actual } => {
                write!(f, "checksum mismatch: expected 0x{expected:02X}, got 0x{actual:02X}")
            }
            Self::UnknownRecordType(t) => write!(f, "unknown record type: 0x{t:02X}"),
            Self::ByteCountMismatch { expected, actual } => {
                write!(f, "byte count mismatch: expected {expected}, got {actual}")
            }
            Self::DataTooLong => write!(f, "data exceeds maximum length"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ParseError {}

/// Parse a hex digit to its value.
const fn parse_hex_digit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'A'..=b'F' => Some(c - b'A' + 10),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

/// Parse two hex characters into a byte.
fn parse_hex_byte(chars: &[u8]) -> Option<u8> {
    if chars.len() < 2 {
        return None;
    }
    let high = parse_hex_digit(chars[0])?;
    let low = parse_hex_digit(chars[1])?;
    Some((high << 4) | low)
}

/// Parse a single Intel HEX record from a line.
///
/// The line may optionally include trailing whitespace (CR, LF).
///
/// # Example
///
/// ```
/// use esc_protocol::ihex::{parse_record, RecordType};
///
/// // Data record at address 0x0100 with 4 bytes
/// let record = parse_record(":04010000AABBCCDDED").unwrap();
/// assert_eq!(record.record_type, RecordType::Data);
/// assert_eq!(record.address, 0x0100);
/// assert_eq!(record.data.as_slice(), &[0xAA, 0xBB, 0xCC, 0xDD]);
///
/// // End of file record
/// let eof = parse_record(":00000001FF").unwrap();
/// assert_eq!(eof.record_type, RecordType::Eof);
/// ```
pub fn parse_record(line: &str) -> Result<Record, ParseError> {
    let line = line.trim_end();
    let bytes = line.as_bytes();

    // Must start with ':'
    if bytes.first() != Some(&b':') {
        return Err(ParseError::MissingStartCode);
    }

    // Minimum: ':' + LL + AAAA + TT + CC = 11 characters
    if bytes.len() < 11 {
        return Err(ParseError::TooShort);
    }

    let hex = &bytes[1..];

    // Parse header fields
    let byte_count = parse_hex_byte(&hex[0..2]).ok_or(ParseError::InvalidHex)? as usize;
    let addr_high = parse_hex_byte(&hex[2..4]).ok_or(ParseError::InvalidHex)?;
    let addr_low = parse_hex_byte(&hex[4..6]).ok_or(ParseError::InvalidHex)?;
    let address = u16::from_be_bytes([addr_high, addr_low]);
    let record_type_byte = parse_hex_byte(&hex[6..8]).ok_or(ParseError::InvalidHex)?;

    let record_type =
        RecordType::from_byte(record_type_byte).ok_or(ParseError::UnknownRecordType(record_type_byte))?;

    // Check we have enough hex chars for data + checksum
    let expected_hex_len = 8 + (byte_count * 2) + 2;
    if hex.len() < expected_hex_len {
        return Err(ParseError::ByteCountMismatch {
            expected: byte_count,
            actual: (hex.len().saturating_sub(10)) / 2,
        });
    }

    // Parse data bytes
    let mut data = RecordData::new();
    for i in 0..byte_count {
        let offset = 8 + (i * 2);
        let byte = parse_hex_byte(&hex[offset..offset + 2]).ok_or(ParseError::InvalidHex)?;
        if !data.push(byte) {
            return Err(ParseError::DataTooLong);
        }
    }

    // Parse and verify checksum
    let checksum_offset = 8 + (byte_count * 2);
    let provided_checksum =
        parse_hex_byte(&hex[checksum_offset..checksum_offset + 2]).ok_or(ParseError::InvalidHex)?;

    // Calculate checksum: two's complement of sum of all bytes
    let mut sum: u8 = 0;
    sum = sum.wrapping_add(byte_count as u8);
    sum = sum.wrapping_add(addr_high);
    sum = sum.wrapping_add(addr_low);
    sum = sum.wrapping_add(record_type_byte);
    for byte in data.as_slice() {
        sum = sum.wrapping_add(*byte);
    }
    let calculated_checksum = (!sum).wrapping_add(1);

    if provided_checksum != calculated_checksum {
        return Err(ParseError::ChecksumMismatch {
            expected: calculated_checksum,
            actual: provided_checksum,
        });
    }

    Ok(Record {
        address,
        record_type,
        data,
    })
}

impl Record {
    /// For ExtendedLinearAddress records, get the upper 16 bits of address.
    ///
    /// Returns `None` if this is not an ExtendedLinearAddress record or data is malformed.
    pub fn extended_linear_address(&self) -> Option<u16> {
        if self.record_type != RecordType::ExtendedLinearAddress {
            return None;
        }
        if self.data.len() != 2 {
            return None;
        }
        Some(u16::from_be_bytes([self.data.as_slice()[0], self.data.as_slice()[1]]))
    }

    /// For ExtendedSegmentAddress records, get the segment address.
    ///
    /// Returns `None` if this is not an ExtendedSegmentAddress record or data is malformed.
    pub fn extended_segment_address(&self) -> Option<u16> {
        if self.record_type != RecordType::ExtendedSegmentAddress {
            return None;
        }
        if self.data.len() != 2 {
            return None;
        }
        Some(u16::from_be_bytes([self.data.as_slice()[0], self.data.as_slice()[1]]))
    }

    /// For StartLinearAddress records, get the 32-bit entry point.
    ///
    /// Returns `None` if this is not a StartLinearAddress record or data is malformed.
    pub fn start_linear_address(&self) -> Option<u32> {
        if self.record_type != RecordType::StartLinearAddress {
            return None;
        }
        if self.data.len() != 4 {
            return None;
        }
        let d = self.data.as_slice();
        Some(u32::from_be_bytes([d[0], d[1], d[2], d[3]]))
    }

    /// Calculate the full 32-bit address for a data record given the current base address.
    ///
    /// For linear addressing (ExtendedLinearAddress), `base_address` is the upper 16 bits << 16.
    /// For segment addressing (ExtendedSegmentAddress), `base_address` is segment << 4.
    pub fn full_address(&self, base_address: u32) -> u32 {
        base_address.wrapping_add(u32::from(self.address))
    }
}

#[cfg(feature = "std")]
pub use std_ext::*;

#[cfg(feature = "std")]
mod std_ext {
    use super::*;
    use std::collections::BTreeMap;
    use std::vec::Vec;

    /// A contiguous segment of firmware data.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Segment {
        /// Start address of this segment.
        pub start_address: u32,
        /// Data bytes.
        pub data: Vec<u8>,
    }

    impl Segment {
        /// End address (exclusive) of this segment.
        pub const fn end_address(&self) -> u32 {
            self.start_address + self.data.len() as u32
        }

        /// Check if an address falls within this segment.
        pub const fn contains(&self, address: u32) -> bool {
            address >= self.start_address && address < self.end_address()
        }
    }

    /// Parsed firmware image from Intel HEX file.
    #[derive(Debug, Clone)]
    pub struct Firmware {
        /// Contiguous data segments, sorted by address.
        pub segments: Vec<Segment>,
        /// Entry point address (from StartLinearAddress record), if present.
        pub entry_point: Option<u32>,
    }

    impl Firmware {
        /// Total size of all segments.
        pub fn total_size(&self) -> usize {
            self.segments.iter().map(|s| s.data.len()).sum()
        }

        /// Get the lowest address in the firmware.
        pub fn min_address(&self) -> Option<u32> {
            self.segments.first().map(|s| s.start_address)
        }

        /// Get the highest address (exclusive) in the firmware.
        pub fn max_address(&self) -> Option<u32> {
            self.segments.last().map(Segment::end_address)
        }

        /// Read a byte at the given address, if present.
        pub fn read_byte(&self, address: u32) -> Option<u8> {
            for segment in &self.segments {
                if segment.contains(address) {
                    let offset = (address - segment.start_address) as usize;
                    return Some(segment.data[offset]);
                }
            }
            None
        }

        /// Flatten segments into contiguous binary starting from lowest address.
        ///
        /// Gaps between segments are filled with 0xFF (empty/erased flash).
        /// Returns an empty Vec if there are no segments.
        pub fn to_binary(&self) -> Vec<u8> {
            if self.segments.is_empty() {
                return Vec::new();
            }

            let min_addr = self.min_address().unwrap();
            let max_addr = self.max_address().unwrap();
            let size = (max_addr - min_addr) as usize;

            let mut binary = std::vec![0xFFu8; size]; // 0xFF = erased flash

            for segment in &self.segments {
                let offset = (segment.start_address - min_addr) as usize;
                binary[offset..offset + segment.data.len()].copy_from_slice(&segment.data);
            }

            binary
        }
    }

    /// Errors that can occur when loading a firmware file.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum LoadError {
        /// Error parsing a specific line.
        Parse { line_number: usize, error: ParseError },
        /// File ended without EOF record.
        MissingEof,
        /// Multiple EOF records found.
        MultipleEof,
    }

    impl std::fmt::Display for LoadError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Parse { line_number, error } => {
                    write!(f, "line {line_number}: {error}")
                }
                Self::MissingEof => write!(f, "missing EOF record"),
                Self::MultipleEof => write!(f, "multiple EOF records"),
            }
        }
    }

    impl std::error::Error for LoadError {}

    /// Load firmware from Intel HEX content.
    ///
    /// Parses all records and assembles them into contiguous segments.
    /// Adjacent data is merged into single segments.
    ///
    /// # Example
    ///
    /// ```
    /// use esc_protocol::ihex::load_firmware;
    ///
    /// let hex = ":020000040000FA\n:04000000AABBCCDDEE\n:00000001FF\n";
    /// let firmware = load_firmware(hex).unwrap();
    /// assert_eq!(firmware.segments.len(), 1);
    /// assert_eq!(firmware.segments[0].start_address, 0);
    /// assert_eq!(firmware.segments[0].data, vec![0xAA, 0xBB, 0xCC, 0xDD]);
    /// ```
    pub fn load_firmware(content: &str) -> Result<Firmware, LoadError> {
        let mut data_map: BTreeMap<u32, u8> = BTreeMap::new();
        let mut base_address: u32 = 0;
        let mut entry_point: Option<u32> = None;
        let mut found_eof = false;

        for (line_idx, line) in content.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            if found_eof {
                // Check if this is another record (not just whitespace)
                if line.starts_with(':') {
                    return Err(LoadError::MultipleEof);
                }
                continue;
            }

            let record = parse_record(line).map_err(|error| LoadError::Parse {
                line_number: line_idx + 1,
                error,
            })?;

            match record.record_type {
                RecordType::Data => {
                    let start = record.full_address(base_address);
                    for (offset, byte) in record.data.as_slice().iter().enumerate() {
                        data_map.insert(start + offset as u32, *byte);
                    }
                }
                RecordType::Eof => {
                    found_eof = true;
                }
                RecordType::ExtendedLinearAddress => {
                    if let Some(upper) = record.extended_linear_address() {
                        base_address = u32::from(upper) << 16;
                    }
                }
                RecordType::ExtendedSegmentAddress => {
                    if let Some(segment) = record.extended_segment_address() {
                        base_address = u32::from(segment) << 4;
                    }
                }
                RecordType::StartLinearAddress => {
                    entry_point = record.start_linear_address();
                }
                RecordType::StartSegmentAddress => {
                    // Rarely used, ignore
                }
            }
        }

        if !found_eof {
            return Err(LoadError::MissingEof);
        }

        // Convert map to segments
        let segments = build_segments(data_map);

        Ok(Firmware {
            segments,
            entry_point,
        })
    }

    /// Build contiguous segments from a map of address -> byte.
    fn build_segments(data_map: BTreeMap<u32, u8>) -> Vec<Segment> {
        let mut segments = Vec::new();
        let mut current_segment: Option<Segment> = None;

        for (address, byte) in data_map {
            match &mut current_segment {
                Some(seg) if seg.end_address() == address => {
                    // Contiguous, extend current segment
                    seg.data.push(byte);
                }
                Some(seg) => {
                    // Gap, start new segment
                    segments.push(core::mem::replace(
                        seg,
                        Segment {
                            start_address: address,
                            data: std::vec![byte],
                        },
                    ));
                }
                None => {
                    current_segment = Some(Segment {
                        start_address: address,
                        data: std::vec![byte],
                    });
                }
            }
        }

        if let Some(seg) = current_segment {
            segments.push(seg);
        }

        segments
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_data_record() {
        // :04 0100 00 AABBCCDD checksum
        // sum = 4 + 1 + 0 + 0 + 0xAA + 0xBB + 0xCC + 0xDD = 0x313
        // checksum = two's complement of 0x13 = 0xED
        let record = parse_record(":04010000AABBCCDDED").unwrap();
        assert_eq!(record.record_type, RecordType::Data);
        assert_eq!(record.address, 0x0100);
        assert_eq!(record.data.as_slice(), &[0xAA, 0xBB, 0xCC, 0xDD]);
    }

    #[test]
    fn parse_eof_record() {
        let record = parse_record(":00000001FF").unwrap();
        assert_eq!(record.record_type, RecordType::Eof);
        assert_eq!(record.address, 0x0000);
        assert!(record.data.is_empty());
    }

    #[test]
    fn parse_extended_linear_address() {
        // Set upper address to 0x0001 (so base = 0x00010000)
        let record = parse_record(":020000040001F9").unwrap();
        assert_eq!(record.record_type, RecordType::ExtendedLinearAddress);
        assert_eq!(record.extended_linear_address(), Some(0x0001));
    }

    #[test]
    fn parse_extended_segment_address() {
        // Segment 0x1000 (base = 0x10000)
        let record = parse_record(":020000021000EC").unwrap();
        assert_eq!(record.record_type, RecordType::ExtendedSegmentAddress);
        assert_eq!(record.extended_segment_address(), Some(0x1000));
    }

    #[test]
    fn parse_start_linear_address() {
        // Entry point at 0x00000100
        let record = parse_record(":0400000500000100F6").unwrap();
        assert_eq!(record.record_type, RecordType::StartLinearAddress);
        assert_eq!(record.start_linear_address(), Some(0x00000100));
    }

    #[test]
    fn parse_real_avr_record() {
        // Real AVR hex record with correct checksum
        // :10 0000 00 0C9434000C9451000C9451000C945100 checksum
        // sum = 0x10 + 0 + 0 + 0 + 0x0C + 0x94 + 0x34 + 0x00 + 0x0C + 0x94 + 0x51 + 0x00
        //     + 0x0C + 0x94 + 0x51 + 0x00 + 0x0C + 0x94 + 0x51 + 0x00 = 0x3B7
        // checksum = two's complement of 0xB7 = 0x49
        let record = parse_record(":100000000C9434000C9451000C9451000C94510049").unwrap();
        assert_eq!(record.record_type, RecordType::Data);
        assert_eq!(record.address, 0x0000);
        assert_eq!(record.data.len(), 16);
        assert_eq!(record.data.as_slice()[0], 0x0C);
        assert_eq!(record.data.as_slice()[1], 0x94);
    }

    #[test]
    fn parse_with_trailing_crlf() {
        let record = parse_record(":00000001FF\r\n").unwrap();
        assert_eq!(record.record_type, RecordType::Eof);
    }

    #[test]
    fn parse_lowercase_hex() {
        let record = parse_record(":04010000aabbccdded").unwrap();
        assert_eq!(record.data.as_slice(), &[0xAA, 0xBB, 0xCC, 0xDD]);
    }

    #[test]
    fn error_missing_start_code() {
        assert_eq!(
            parse_record("04010000AABBCCDD5C"),
            Err(ParseError::MissingStartCode)
        );
    }

    #[test]
    fn error_too_short() {
        assert_eq!(parse_record(":0000"), Err(ParseError::TooShort));
    }

    #[test]
    fn error_invalid_hex() {
        assert_eq!(parse_record(":0G000001FF"), Err(ParseError::InvalidHex));
    }

    #[test]
    fn error_checksum_mismatch() {
        let result = parse_record(":00000001FE");
        assert!(matches!(result, Err(ParseError::ChecksumMismatch { .. })));
    }

    #[test]
    fn error_unknown_record_type() {
        // Type 0x06 doesn't exist
        // :00 0000 06 checksum
        // checksum = ~(0 + 0 + 0 + 6) + 1 = ~6 + 1 = 0xF9 + 1 = 0xFA
        let result = parse_record(":00000006FA");
        assert_eq!(result, Err(ParseError::UnknownRecordType(0x06)));
    }

    #[test]
    fn full_address_calculation() {
        let record = parse_record(":04010000AABBCCDDED").unwrap();
        // With base 0x00010000, address should be 0x00010100
        assert_eq!(record.full_address(0x0001_0000), 0x0001_0100);
    }

    #[test]
    #[cfg(feature = "std")]
    fn load_simple_firmware() {
        // :04 0000 00 AABBCCDD checksum
        // sum = 4 + 0 + 0 + 0 + 0xAA + 0xBB + 0xCC + 0xDD = 0x312, checksum = 0xEE
        let hex = ":020000040000FA\n:04000000AABBCCDDEE\n:00000001FF\n";
        let firmware = load_firmware(hex).unwrap();
        assert_eq!(firmware.segments.len(), 1);
        assert_eq!(firmware.segments[0].start_address, 0);
        assert_eq!(firmware.segments[0].data, std::vec![0xAA, 0xBB, 0xCC, 0xDD]);
    }

    #[test]
    #[cfg(feature = "std")]
    fn load_firmware_with_gap() {
        // :02 0000 00 AABB -> sum = 2+0+0+0+0xAA+0xBB = 0x167, checksum = 0x99
        // :02 0010 00 CCDD -> sum = 2+0+0x10+0+0xCC+0xDD = 0x1BB, checksum = 0x45
        let hex = concat!(
            ":020000040000FA\n",      // Base 0x00000000
            ":02000000AABB99\n",      // 2 bytes at 0x0000
            ":02001000CCDD45\n",      // 2 bytes at 0x0010 (gap!)
            ":00000001FF\n"
        );
        let firmware = load_firmware(hex).unwrap();
        assert_eq!(firmware.segments.len(), 2);
        assert_eq!(firmware.segments[0].start_address, 0x0000);
        assert_eq!(firmware.segments[0].data, std::vec![0xAA, 0xBB]);
        assert_eq!(firmware.segments[1].start_address, 0x0010);
        assert_eq!(firmware.segments[1].data, std::vec![0xCC, 0xDD]);
    }

    #[test]
    #[cfg(feature = "std")]
    fn load_firmware_extended_address() {
        // :02 0000 00 AABB -> checksum = 0x99
        let hex = concat!(
            ":020000040001F9\n",      // Base 0x00010000
            ":02000000AABB99\n",      // 2 bytes at 0x00010000
            ":00000001FF\n"
        );
        let firmware = load_firmware(hex).unwrap();
        assert_eq!(firmware.segments[0].start_address, 0x0001_0000);
    }

    #[test]
    #[cfg(feature = "std")]
    fn load_firmware_with_entry_point() {
        let hex = concat!(
            ":020000040000FA\n",
            ":04000000AABBCCDDEE\n",
            ":0400000500001000E7\n",  // Entry point 0x00001000
            ":00000001FF\n"
        );
        let firmware = load_firmware(hex).unwrap();
        assert_eq!(firmware.entry_point, Some(0x0000_1000));
    }

    #[test]
    #[cfg(feature = "std")]
    fn load_firmware_missing_eof() {
        let hex = ":020000040000FA\n:04000000AABBCCDDEE\n";
        let result = load_firmware(hex);
        assert!(matches!(result, Err(LoadError::MissingEof)));
    }

    #[test]
    #[cfg(feature = "std")]
    fn load_firmware_read_byte() {
        let hex = ":020000040000FA\n:04000000AABBCCDDEE\n:00000001FF\n";
        let firmware = load_firmware(hex).unwrap();
        assert_eq!(firmware.read_byte(0), Some(0xAA));
        assert_eq!(firmware.read_byte(1), Some(0xBB));
        assert_eq!(firmware.read_byte(2), Some(0xCC));
        assert_eq!(firmware.read_byte(3), Some(0xDD));
        assert_eq!(firmware.read_byte(4), None);
        assert_eq!(firmware.read_byte(100), None);
    }

    #[test]
    #[cfg(feature = "std")]
    fn firmware_to_binary_simple() {
        let hex = ":020000040000FA\n:04000000AABBCCDDEE\n:00000001FF\n";
        let firmware = load_firmware(hex).unwrap();
        let binary = firmware.to_binary();
        assert_eq!(binary, std::vec![0xAA, 0xBB, 0xCC, 0xDD]);
    }

    #[test]
    #[cfg(feature = "std")]
    fn firmware_to_binary_with_gap() {
        // Two segments with a gap between them
        let hex = concat!(
            ":020000040000FA\n",      // Base 0x00000000
            ":02000000AABB99\n",      // 2 bytes at 0x0000
            ":02001000CCDD45\n",      // 2 bytes at 0x0010 (gap!)
            ":00000001FF\n"
        );
        let firmware = load_firmware(hex).unwrap();
        let binary = firmware.to_binary();

        // Should be 18 bytes total (from 0x0000 to 0x0012)
        assert_eq!(binary.len(), 18);
        // First 2 bytes are data
        assert_eq!(&binary[0..2], &[0xAA, 0xBB]);
        // Gap filled with 0xFF
        assert!(binary[2..16].iter().all(|&b| b == 0xFF));
        // Last 2 bytes are data
        assert_eq!(&binary[16..18], &[0xCC, 0xDD]);
    }

    #[test]
    #[cfg(feature = "std")]
    fn firmware_to_binary_empty() {
        // Edge case: firmware with no segments
        // This is unusual but should work
        let firmware = Firmware {
            segments: std::vec![],
            entry_point: None,
        };
        let binary = firmware.to_binary();
        assert!(binary.is_empty());
    }

    // =========================================================================
    // Flash/Verify simulation tests
    // =========================================================================

    #[test]
    #[cfg(feature = "std")]
    fn firmware_chunking_128_bytes() {
        // Simulate how flash command chunks firmware
        let firmware = Firmware {
            segments: std::vec![Segment {
                start_address: 0x1000,
                data: (0u8..=255).cycle().take(512).collect(), // 512 bytes
            }],
            entry_point: None,
        };
        let binary = firmware.to_binary();

        assert_eq!(binary.len(), 512);

        // Chunk like flash command does (128-byte chunks)
        let chunk_size = 128;
        let chunks: std::vec::Vec<_> = binary.chunks(chunk_size).collect();

        assert_eq!(chunks.len(), 4); // 512 / 128 = 4 chunks
        assert_eq!(chunks[0].len(), 128);
        assert_eq!(chunks[3].len(), 128);

        // Verify chunk addresses
        let firmware_start: u16 = 0x1000;
        for (i, _chunk) in chunks.iter().enumerate() {
            let addr = firmware_start + (i * chunk_size) as u16;
            assert_eq!(addr, 0x1000 + (i as u16 * 128));
        }
    }

    #[test]
    #[cfg(feature = "std")]
    fn firmware_partial_last_chunk() {
        // Test firmware that doesn't align to chunk size
        let firmware = Firmware {
            segments: std::vec![Segment {
                start_address: 0x1000,
                data: std::vec![0xAA; 300], // 300 bytes - not aligned
            }],
            entry_point: None,
        };
        let binary = firmware.to_binary();

        // 256-byte chunks: first full, second partial
        let chunk_size = 256;
        let chunks: std::vec::Vec<_> = binary.chunks(chunk_size).collect();

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 256);
        assert_eq!(chunks[1].len(), 44); // 300 - 256 = 44
    }

    #[test]
    #[cfg(feature = "std")]
    fn firmware_verification_comparison() {
        // Simulate verification: compare expected vs actual
        let firmware = Firmware {
            segments: std::vec![Segment {
                start_address: 0x1000,
                data: std::vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08],
            }],
            entry_point: None,
        };
        let expected = firmware.to_binary();

        // Simulate reading back from ESC (matching)
        let actual_ok = std::vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        assert_eq!(expected, actual_ok);

        // Simulate mismatch
        let actual_bad = std::vec![0x01, 0x02, 0xFF, 0x04, 0x05, 0x06, 0x07, 0x08];
        assert_ne!(expected, actual_bad);

        // Find mismatch position
        let mismatch_pos = expected
            .iter()
            .zip(actual_bad.iter())
            .position(|(a, b)| a != b);
        assert_eq!(mismatch_pos, Some(2)); // Byte 2 differs
    }

    #[test]
    #[cfg(feature = "std")]
    fn buffer_size_encoding() {
        // Test that 256 bytes should be encoded as 0 in protocol
        let size_256: u8 = 0; // 256 encoded as 0
        let size_128: u8 = 128;

        // When we have 256 bytes, we send 0
        let data_256 = [0u8; 256];
        let encoded = if data_256.len() == 256 { 0u8 } else { data_256.len() as u8 };
        assert_eq!(encoded, size_256);

        // When we have 128 bytes, we send 128
        let data_128 = [0u8; 128];
        let encoded = if data_128.len() == 256 { 0u8 } else { data_128.len() as u8 };
        assert_eq!(encoded, size_128);
    }
}
