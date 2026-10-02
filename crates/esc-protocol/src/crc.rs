/// CRC-16/IBM implementation for BLHeli protocol.
///
/// - Polynomial: 0xA001
/// - Initial value: 0x0000
/// - Output: LSB first, then MSB
///
/// Reference implementation from BLHeli protocol documentation.
pub const fn crc16_ibm(buf: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    let mut i = 0;
    while i < buf.len() {
        let mut xb = buf[i];
        let mut bit = 0;
        while bit < 8 {
            if ((xb & 0x01) ^ (crc as u8 & 0x01)) != 0 {
                crc >>= 1;
                crc ^= 0xA001;
            } else {
                crc >>= 1;
            }
            xb >>= 1;
            bit += 1;
        }
        i += 1;
    }
    crc
}

/// Returns CRC bytes in wire order (LSB first, then MSB).
#[inline]
pub const fn crc16_ibm_bytes(buf: &[u8]) -> [u8; 2] {
    let crc = crc16_ibm(buf);
    [crc as u8, (crc >> 8) as u8]
}

/// CRC-16/IBM polynomial constant.
pub const CRC_POLY: u16 = 0xA001;

/// BLHeli config block size in bytes.
pub const BLHELI_CONFIG_BLOCK_SIZE: usize = 256;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_blheli_string() {
        let data = b"BLHeli";
        assert_eq!(crc16_ibm(data), 0x7DF4);
    }

    #[test]
    fn crc_keepalive() {
        let data = [0xFD, 0x00];
        assert_eq!(crc16_ibm(&data), 0x9040);
    }

    #[test]
    fn crc_disconnect() {
        let data = [0x00, 0x01];
        assert_eq!(crc16_ibm(&data), 0xC0C1);
    }

    #[test]
    fn crc_set_address_7c00() {
        let data = [0xFF, 0x00, 0x7C, 0x00];
        assert_eq!(crc16_ibm(&data), 0xD410);
    }

    #[test]
    fn crc_set_address_eb00() {
        let data = [0xFF, 0x00, 0xEB, 0x00];
        assert_eq!(crc16_ibm(&data), 0xE47E);
    }

    #[test]
    fn crc_read_256() {
        let data = [0x03, 0x00];
        assert_eq!(crc16_ibm(&data), 0xF000);
    }

    #[test]
    fn crc_read_16() {
        // Wire format: [03 10 01 3C] - CRC bytes are [01 3C] (LSB first)
        // So CRC value is 0x3C01
        let data = [0x03, 0x10];
        assert_eq!(crc16_ibm(&data), 0x3C01);
    }

    #[test]
    fn crc_write_header() {
        let data = [0xFE, 0x00, 0x01, 0x00];
        assert_eq!(crc16_ibm(&data), 0x7830);
    }

    #[test]
    fn crc_commit_flash() {
        let data = [0x01, 0x01];
        assert_eq!(crc16_ibm(&data), 0x50C0);
    }

    #[test]
    fn crc_erase_page() {
        let data = [0x02, 0x00];
        assert_eq!(crc16_ibm(&data), 0x6001);
    }

    #[test]
    fn crc_bytes_wire_order() {
        // "BLHeli" -> 0x7DF4 -> [0xF4, 0x7D] in wire order
        let bytes = crc16_ibm_bytes(b"BLHeli");
        assert_eq!(bytes, [0xF4, 0x7D]);
    }

    #[test]
    fn crc_write_page_short() {
        // Short write command (2 bytes)
        // CRC of [FE 00] = 0x6040 -> wire bytes [40 60]
        let data = [0xFE, 0x00];
        assert_eq!(crc16_ibm(&data), 0x6040);
    }
}
