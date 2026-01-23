//! DShot CRC-4 implementation.
//!
//! The CRC-4 is calculated over the 12-bit data (throttle + telemetry flag).
//! It's a simple XOR-based checksum of the three nibbles.

/// Calculate DShot CRC-4 checksum.
///
/// The CRC is computed by XORing all three nibbles of the 12-bit data:
/// `(data ^ (data >> 4) ^ (data >> 8)) & 0x0F`
///
/// # Arguments
/// * `data` - 12-bit value (bits 11:0 used, upper bits ignored)
///
/// # Returns
/// 4-bit CRC value (0-15)
#[inline]
pub const fn crc4(data: u16) -> u8 {
    let data = data & 0x0FFF; // Mask to 12 bits
    ((data ^ (data >> 4) ^ (data >> 8)) & 0x0F) as u8
}

/// Verify CRC-4 checksum of a complete 16-bit DShot frame.
///
/// # Arguments
/// * `frame` - Complete 16-bit DShot frame (12-bit data + 4-bit CRC)
///
/// # Returns
/// `true` if CRC is valid
#[inline]
pub const fn verify_crc4(frame: u16) -> bool {
    let data = frame >> 4;        // Upper 12 bits
    let crc = (frame & 0x0F) as u8; // Lower 4 bits
    crc4(data) == crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc4_zero() {
        // All zeros should give CRC 0
        assert_eq!(crc4(0x000), 0);
    }

    #[test]
    fn crc4_all_ones() {
        // 0xFFF: nibbles are F, F, F -> F ^ F ^ F = F
        assert_eq!(crc4(0xFFF), 0x0F);
    }

    #[test]
    fn crc4_single_nibble() {
        // 0x00A: nibbles are 0, 0, A -> 0 ^ 0 ^ A = A
        assert_eq!(crc4(0x00A), 0x0A);
    }

    #[test]
    fn crc4_mixed() {
        // 0x123: nibbles are 1, 2, 3 -> 1 ^ 2 ^ 3 = 0
        assert_eq!(crc4(0x123), 0x00);

        // 0x456: nibbles are 4, 5, 6 -> 4 ^ 5 ^ 6 = 7
        assert_eq!(crc4(0x456), 0x07);

        // 0xABC: nibbles are A, B, C -> A ^ B ^ C = 0xD (13)
        // 1010 ^ 1011 ^ 1100 = 0001 ^ 1100 = 1101
        assert_eq!(crc4(0xABC), 0x0D);
    }

    #[test]
    fn crc4_ignores_upper_bits() {
        // Upper 4 bits should be ignored
        assert_eq!(crc4(0xF123), crc4(0x0123));
    }

    #[test]
    fn verify_valid_frame() {
        // Construct frame: data=0x123, crc=0 -> frame = 0x1230
        let data: u16 = 0x123;
        let crc = crc4(data);
        let frame = (data << 4) | (crc as u16);

        assert!(verify_crc4(frame));
    }

    #[test]
    fn verify_invalid_frame() {
        // Frame with wrong CRC
        let frame: u16 = 0x1231; // data=0x123, crc should be 0, but we put 1
        assert!(!verify_crc4(frame));
    }

    #[test]
    fn crc4_is_const() {
        // Verify const fn works at compile time
        const CRC: u8 = crc4(0x456);
        assert_eq!(CRC, 0x07);
    }
}
