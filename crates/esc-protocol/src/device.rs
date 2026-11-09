//! Device classification from the connect reply.
//!
//! The reply is `"471" PIN FLASH ...`, and the historical "device id" is
//! `PIN | FLASH << 8`. AM32 bootloaders put the signal pin code (`port << 4 | pin`)
//! in the low byte and a flash size code in the high byte, so an AM32 ESC is
//! recognised by its flash code, whatever pin it uses.
//!
//! **Supported:** AM32.
//!
//! **Not supported:** BLHeli_S (SiLabs EFM8) and ATmega bootloaders are recognised
//! so the error can name them; BLHeli_32 and anything else is "unknown".

/// Known SIL (Silabs) bootloader device IDs - these are BLHeli_S (unsupported).
pub const SIL_DEVICE_IDS: &[u16] = &[0xF310, 0xF330, 0xF410, 0xF390, 0xF850, 0xE8B1, 0xE8B2];

/// Known ATM (AtMega) bootloader device IDs - legacy, not supported.
pub const ATM_DEVICE_IDS: &[u16] = &[0x9307, 0x930A, 0x930F, 0x940B];

/// AM32 flash size codes: 32K, 64K, 128K, and NXP 64K / 128K.
///
/// The NXP codes have no fixed layout; they need the bootloader's devinfo.
pub const AM32_FLASH_CODES: &[u8] = &[0x1F, 0x35, 0x2B, 0x15, 0x16];

/// Bootloader type based on MCU architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootloaderType {
    /// Silabs EFM8 bootloader (BLHeli_S, unsupported).
    Sil,
    /// ARM Cortex-M bootloader with an AM32 flash code.
    Arm,
    /// AtMega bootloader (legacy, unsupported).
    Atm,
    /// Unknown bootloader type.
    Unknown,
}

/// Supported firmware types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirmwareType {
    /// AM32 firmware (runs on ARM MCUs).
    Am32,
}

/// Device classification result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Raw device ID from connect response (`PIN | FLASH << 8`).
    pub device_id: u16,
    /// Bootloader type.
    pub bootloader: BootloaderType,
    /// Inferred firmware type (if supported).
    pub firmware: Option<FirmwareType>,
}

impl DeviceInfo {
    /// Returns true if this device is supported.
    #[inline]
    pub const fn is_supported(&self) -> bool {
        self.firmware.is_some()
    }

    /// Signal pin code (`port << 4 | pin` on AM32: `0x06` is PA6, `0x14` PB4).
    pub const fn pin_code(&self) -> u8 {
        self.device_id as u8
    }

    /// Flash size code (`0x1F` 32K, `0x35` 64K, `0x2B` 128K on AM32).
    pub const fn flash_code(&self) -> u8 {
        (self.device_id >> 8) as u8
    }
}

/// Classify a device by its ID from the connect response.
pub fn classify_device(device_id: u16) -> DeviceInfo {
    let (bootloader, firmware) = if SIL_DEVICE_IDS.contains(&device_id) {
        (BootloaderType::Sil, None)
    } else if ATM_DEVICE_IDS.contains(&device_id) {
        (BootloaderType::Atm, None)
    } else if AM32_FLASH_CODES.contains(&((device_id >> 8) as u8)) {
        (BootloaderType::Arm, Some(FirmwareType::Am32))
    } else {
        (BootloaderType::Unknown, None)
    };
    DeviceInfo {
        device_id,
        bootloader,
        firmware,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn am32_recognised_on_any_pin() {
        // F7: PA6, PB4, PA15, PA0, PB2, PC6, PA2 on each flash size.
        for pin in [0x06u8, 0x14, 0x0F, 0x00, 0x12, 0x26, 0x02] {
            for flash in [0x1Fu8, 0x35, 0x2B] {
                let info = classify_device(u16::from_le_bytes([pin, flash]));
                assert_eq!(info.bootloader, BootloaderType::Arm);
                assert_eq!(info.firmware, Some(FirmwareType::Am32));
                assert_eq!((info.pin_code(), info.flash_code()), (pin, flash));
            }
        }
    }

    #[test]
    fn flash_codes_that_am32_does_not_use_are_unknown() {
        for id in [0x3306u16, 0x3406, 0x4706, 0xDEAD] {
            let info = classify_device(id);
            assert_eq!(info.bootloader, BootloaderType::Unknown, "0x{id:04X}");
            assert!(!info.is_supported());
        }
    }

    #[test]
    fn sil_and_atm_are_recognised_and_unsupported() {
        for &id in SIL_DEVICE_IDS {
            let info = classify_device(id);
            assert_eq!(info.bootloader, BootloaderType::Sil);
            assert!(!info.is_supported());
        }
        for &id in ATM_DEVICE_IDS {
            let info = classify_device(id);
            assert_eq!(info.bootloader, BootloaderType::Atm);
            assert!(!info.is_supported());
        }
    }
}
