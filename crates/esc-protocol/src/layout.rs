//! Where things live in an AM32 ESC's flash, in Set Address units.
//!
//! The bootloader takes 16-bit addresses. On 128 KiB parts it shifts them left by 2
//! before use, so a protocol address is `byte_offset >> address_shift`. Bootloaders
//! speaking protocol version 3+ publish the layout in a `devinfo` structure (read at
//! [`crate::packet::ADDR_MAGIC_DEVINFO`]); older ones are covered by
//! [`Layout::from_flash_code`].

/// `devinfo.magic1`, little-endian at offset 0.
pub const DEVINFO_MAGIC1: u32 = 0x5925_e3da;
/// `devinfo.magic2`, little-endian at offset 4.
pub const DEVINFO_MAGIC2: u32 = 0x4eb8_63d9;
/// Bytes to read at `ADDR_MAGIC_DEVINFO`: magics (8), deviceInfo (9), length,
/// address shift, then four `u16` start addresses.
pub const DEVINFO_LEN: usize = 27;

/// Flash layout of a connected ESC, in protocol (Set Address) units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// Left shift the bootloader applies to Set Address values (0 or 2).
    pub address_shift: u8,
    /// Application start (`0x1000` bytes, or `0x4000` with a DroneCAN bootloader).
    pub firmware_start: u16,
    /// 32-byte firmware name.
    pub filename_start: u16,
    /// EEPROM (settings) base.
    pub eeprom_start: u16,
}

impl Layout {
    /// Layout of a non-CAN AM32 bootloader, from the connect reply's flash code.
    ///
    /// `0x1F` 32 KiB, `0x35` 64 KiB, `0x2B` 128 KiB (shift 2). `None` for anything
    /// else, including NXP parts, whose EEPROM position depends on the sector size:
    /// those need [`Self::parse_devinfo`].
    pub const fn from_flash_code(flash_code: u8) -> Option<Self> {
        // (shift, EEPROM byte offset); the app starts at 0x1000 bytes.
        let (shift, eeprom) = match flash_code {
            0x1F => (0, 0x7C00),
            0x35 => (0, 0xF800),
            0x2B => (2, 0x1_F800),
            _ => return None,
        };
        Some(Self {
            address_shift: shift,
            firmware_start: (0x1000u32 >> shift) as u16,
            filename_start: ((eeprom - 32) >> shift) as u16,
            eeprom_start: (eeprom >> shift) as u16,
        })
    }

    /// Parse the devinfo structure. `None` if the magics or length do not match.
    pub fn parse_devinfo(data: &[u8]) -> Option<Self> {
        let d = data.get(..DEVINFO_LEN)?;
        let u32_at = |i: usize| u32::from_le_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]);
        let u16_at = |i: usize| u16::from_le_bytes([d[i], d[i + 1]]);
        if u32_at(0) != DEVINFO_MAGIC1 || u32_at(4) != DEVINFO_MAGIC2 {
            return None;
        }
        if usize::from(d[17]) < DEVINFO_LEN || d[18] > 4 {
            return None;
        }
        Some(Self {
            address_shift: d[18],
            firmware_start: u16_at(19),
            filename_start: u16_at(21),
            eeprom_start: u16_at(23),
        })
    }

    /// Bytes covered by one protocol address unit.
    pub const fn unit(&self) -> u32 {
        1 << self.address_shift
    }

    /// Protocol address `bytes` past `base`, or `None` if `bytes` is not a whole
    /// number of units or the result does not fit in 16 bits.
    pub const fn offset(&self, base: u16, bytes: u32) -> Option<u16> {
        if !bytes.is_multiple_of(self.unit()) {
            return None;
        }
        let units = bytes >> self.address_shift;
        // Checked: a wrapped sum could land on a magic address (0x20 is the EEPROM).
        match (base as u32).checked_add(units) {
            Some(addr) if addr <= u16::MAX as u32 => Some(addr as u16),
            _ => None,
        }
    }

    /// Byte offset of the application from the start of flash.
    pub const fn firmware_byte_offset(&self) -> u32 {
        (self.firmware_start as u32) << self.address_shift
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_does_not_wrap() {
        let f051 = Layout::from_flash_code(0x1F).unwrap();
        // 0x1000 + 0xFFFF_F020 wraps to 0x20, the EEPROM magic.
        assert_eq!(f051.offset(0x1000, 0xFFFF_F020), None);
        assert_eq!(f051.offset(0x1000, 0x100), Some(0x1100));
    }

    #[test]
    fn flash_code_layouts() {
        let f051 = Layout::from_flash_code(0x1F).unwrap();
        assert_eq!(
            (f051.firmware_start, f051.filename_start, f051.eeprom_start),
            (0x1000, 0x7BE0, 0x7C00)
        );

        let f421 = Layout::from_flash_code(0x35).unwrap();
        assert_eq!(f421.eeprom_start, 0xF800);

        // F5/F6: G071 addresses are divided by 4.
        let g071 = Layout::from_flash_code(0x2B).unwrap();
        assert_eq!(g071.address_shift, 2);
        assert_eq!(g071.firmware_start, 0x0400);
        assert_eq!(g071.eeprom_start, 0x7E00);
        assert_eq!(g071.filename_start, 0x7DF8);
        assert_eq!(g071.firmware_byte_offset(), 0x1000);

        assert_eq!(Layout::from_flash_code(0x33), None);
    }

    #[test]
    fn offsets_respect_the_shift() {
        let g071 = Layout::from_flash_code(0x2B).unwrap();
        assert_eq!(g071.offset(g071.firmware_start, 256), Some(0x0440));
        assert_eq!(g071.offset(g071.firmware_start, 2), None);
        assert_eq!(g071.offset(0xFFFF, 4), None);

        let f051 = Layout::from_flash_code(0x1F).unwrap();
        assert_eq!(f051.offset(0x1000, 256), Some(0x1100));
    }

    fn devinfo(shift: u8, fw: u16, name: u16, eeprom: u16) -> [u8; DEVINFO_LEN] {
        let mut d = [0u8; DEVINFO_LEN];
        d[0..4].copy_from_slice(&DEVINFO_MAGIC1.to_le_bytes());
        d[4..8].copy_from_slice(&DEVINFO_MAGIC2.to_le_bytes());
        d[8..17].copy_from_slice(b"471\x06\x2B\x06\x06\x03\x30");
        d[17] = DEVINFO_LEN as u8;
        d[18] = shift;
        d[19..21].copy_from_slice(&fw.to_le_bytes());
        d[21..23].copy_from_slice(&name.to_le_bytes());
        d[23..25].copy_from_slice(&eeprom.to_le_bytes());
        d[25..27].copy_from_slice(&(eeprom + 12).to_le_bytes());
        d
    }

    #[test]
    fn devinfo_parses_dronecan_layout() {
        // DroneCAN bootloader on G071: app at 0x4000 bytes.
        let d = devinfo(2, 0x1000, 0x1400, 0x7E00);
        let l = Layout::parse_devinfo(&d).unwrap();
        assert_eq!(l.firmware_start, 0x1000);
        assert_eq!(l.firmware_byte_offset(), 0x4000);
        assert_eq!(l.filename_start, 0x1400);
    }

    #[test]
    fn devinfo_rejects_bad_magic_or_short() {
        let mut d = devinfo(0, 0x1000, 0x7BE0, 0x7C00);
        assert!(Layout::parse_devinfo(&d[..20]).is_none());
        d[0] ^= 1;
        assert!(Layout::parse_devinfo(&d).is_none());
    }
}
