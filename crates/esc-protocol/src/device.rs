//! Device type classification.
//!
//! Classifies ESC devices by bootloader type and firmware.
//!
//! **Supported:**
//! - BLHeli_S (SIL/Silabs EFM8 MCUs)
//! - AM32 (ARM MCUs - AT32, STM32, etc.)
//!
//! **NOT Supported:**
//! - BLHeli_32 (will panic)
//! - ATmega-based ESCs (legacy)

/// Known SIL (Silabs) bootloader device IDs - these are BLHeli_S.
pub const SIL_DEVICE_IDS: &[u16] = &[0xF310, 0xF330, 0xF410, 0xF390, 0xF850, 0xE8B1, 0xE8B2];

/// Known ARM bootloader device IDs - these could be AM32 or BLHeli_32.
/// We assume AM32; BLHeli_32 detection requires firmware inspection.
pub const ARM_DEVICE_IDS: &[u16] = &[0x1F02, 0x1F06, 0x3306, 0x3406, 0x3506, 0x2B06, 0x4706];

/// Known ATM (AtMega) bootloader device IDs - legacy, not supported.
pub const ATM_DEVICE_IDS: &[u16] = &[0x9307, 0x930A, 0x930F, 0x940B];

/// Bootloader type based on MCU architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootloaderType {
    /// Silabs EFM8 bootloader (BLHeli_S).
    Sil,
    /// ARM Cortex-M bootloader (AM32 or BLHeli_32).
    Arm,
    /// AtMega bootloader (legacy, unsupported).
    Atm,
    /// Unknown bootloader type.
    Unknown,
}

/// Supported firmware types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirmwareType {
    /// BLHeli_S firmware (runs on Silabs EFM8).
    BlheliS,
    /// AM32 firmware (runs on ARM MCUs).
    Am32,
}

/// Device classification result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Raw device ID from connect response.
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

    /// Get the EEPROM/config base address for this device.
    ///
    /// The high byte of the device_id corresponds to the MCU type:
    /// - 0x1F (STM32F051): 0x7C00
    /// - 0x2B (STM32G071): 0x7E00
    /// - 0x35 (AT32F421/STM32F3xx): 0xF800
    /// - Others (SiLabs, GD32, etc.): 0x7C00 (default)
    pub const fn eeprom_offset(&self) -> u16 {
        let mcu_type = (self.device_id >> 8) as u8;
        match mcu_type {
            0x2B => 0x7E00, // STM32G071
            0x35 => 0xF800, // AT32F421 / STM32F3xx
            _ => 0x7C00,    // STM32F051, SiLabs, and others
        }
    }
}

/// Classify a device by its ID from the connect response.
///
/// # Panics
///
/// This function does NOT panic. Use [`ensure_supported`] to panic on
/// unsupported devices.
pub fn classify_device(device_id: u16) -> DeviceInfo {
    // Check SIL devices first (BLHeli_S)
    if SIL_DEVICE_IDS.contains(&device_id) {
        return DeviceInfo {
            device_id,
            bootloader: BootloaderType::Sil,
            firmware: Some(FirmwareType::BlheliS),
        };
    }

    // Check ARM devices (assume AM32, NOT BLHeli_32)
    if ARM_DEVICE_IDS.contains(&device_id) {
        return DeviceInfo {
            device_id,
            bootloader: BootloaderType::Arm,
            firmware: Some(FirmwareType::Am32),
        };
    }

    // Check ATM devices (legacy, unsupported)
    if ATM_DEVICE_IDS.contains(&device_id) {
        return DeviceInfo {
            device_id,
            bootloader: BootloaderType::Atm,
            firmware: None,
        };
    }

    // Unknown device
    DeviceInfo {
        device_id,
        bootloader: BootloaderType::Unknown,
        firmware: None,
    }
}

/// Classify device and panic if unsupported.
///
/// # Panics
///
/// Panics if the device is not a supported BLHeli_S or AM32 ESC.
pub fn ensure_supported(device_id: u16) -> DeviceInfo {
    let info = classify_device(device_id);

    match info.bootloader {
        BootloaderType::Sil => info,
        BootloaderType::Arm => info,
        BootloaderType::Atm => {
            panic!(
                "Unsupported device: ATmega-based ESC (device_id=0x{:04X}). \
                 Only BLHeli_S and AM32 are supported.",
                device_id
            );
        }
        BootloaderType::Unknown => {
            panic!(
                "Unknown device (device_id=0x{:04X}). \
                 This might be BLHeli_32 which is NOT supported. \
                 Only BLHeli_S and AM32 are supported.",
                device_id
            );
        }
    }
}

/// Check if device ID looks like BLHeli_32.
///
/// BLHeli_32 uses ARM bootloader but has different device IDs than AM32.
/// This is a heuristic - for definitive detection, check firmware signature.
///
/// Note: Currently returns false for all known AM32 IDs. Unknown ARM devices
/// should be treated with caution.
pub const fn might_be_blheli32(device_id: u16) -> bool {
    // Known AM32 device IDs are in ARM_DEVICE_IDS
    // If it's ARM but not in our list, it might be BLHeli_32
    // This is conservative - we'd rather reject than brick
    let is_known_arm = matches!(
        device_id,
        0x1F02 | 0x1F06 | 0x3306 | 0x3406 | 0x3506 | 0x2B06 | 0x4706
    );

    // If bootloader looks ARM-ish but not in our known list, suspect BLHeli_32
    !is_known_arm && (device_id & 0x00FF) == 0x06
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_sil_device() {
        let info = classify_device(0xF390);
        assert_eq!(info.bootloader, BootloaderType::Sil);
        assert_eq!(info.firmware, Some(FirmwareType::BlheliS));
        assert!(info.is_supported());
    }

    #[test]
    fn classify_arm_device() {
        let info = classify_device(0x3306);
        assert_eq!(info.bootloader, BootloaderType::Arm);
        assert_eq!(info.firmware, Some(FirmwareType::Am32));
        assert!(info.is_supported());
    }

    #[test]
    fn classify_atm_device() {
        let info = classify_device(0x930A);
        assert_eq!(info.bootloader, BootloaderType::Atm);
        assert_eq!(info.firmware, None);
        assert!(!info.is_supported());
    }

    #[test]
    fn classify_unknown_device() {
        let info = classify_device(0x1234);
        assert_eq!(info.bootloader, BootloaderType::Unknown);
        assert_eq!(info.firmware, None);
        assert!(!info.is_supported());
    }

    #[test]
    fn ensure_supported_sil() {
        let info = ensure_supported(0xE8B1);
        assert_eq!(info.firmware, Some(FirmwareType::BlheliS));
    }

    #[test]
    fn ensure_supported_arm() {
        let info = ensure_supported(0x4706);
        assert_eq!(info.firmware, Some(FirmwareType::Am32));
    }

    #[test]
    #[should_panic(expected = "ATmega-based ESC")]
    fn ensure_supported_panics_on_atm() {
        ensure_supported(0x9307);
    }

    #[test]
    #[should_panic(expected = "Unknown device")]
    fn ensure_supported_panics_on_unknown() {
        ensure_supported(0xDEAD);
    }

    #[test]
    fn blheli32_heuristic() {
        // Known AM32 devices should NOT be flagged
        assert!(!might_be_blheli32(0x3306));
        assert!(!might_be_blheli32(0x4706));

        // Unknown device ending in 0x06 might be BLHeli_32
        assert!(might_be_blheli32(0x9906));

        // Random device should not match
        assert!(!might_be_blheli32(0x1234));
    }

    // Compile-time verification of eeprom_offset mapping.
    // eeprom_offset only depends on device_id, so we can construct DeviceInfo directly.
    const _: () = {
        const fn eeprom(device_id: u16) -> u16 {
            DeviceInfo {
                device_id,
                bootloader: BootloaderType::Arm,
                firmware: None,
            }
            .eeprom_offset()
        }
        // STM32F051 variants → 0x7C00
        assert!(eeprom(0x1F02) == 0x7C00);
        assert!(eeprom(0x1F06) == 0x7C00);
        // STM32G071 → 0x7E00
        assert!(eeprom(0x2B06) == 0x7E00);
        // AT32F421 / STM32F3xx → 0xF800
        assert!(eeprom(0x3506) == 0xF800);
        // Other ARM devices default to 0x7C00
        assert!(eeprom(0x3306) == 0x7C00);
        assert!(eeprom(0x4706) == 0x7C00);
        // SiLabs devices → 0x7C00
        assert!(eeprom(0xF390) == 0x7C00);
    };

    #[test]
    fn all_known_sil_devices_supported() {
        for &id in SIL_DEVICE_IDS {
            let info = classify_device(id);
            assert!(
                info.is_supported(),
                "SIL device 0x{:04X} should be supported",
                id
            );
            assert_eq!(info.firmware, Some(FirmwareType::BlheliS));
        }
    }

    #[test]
    fn all_known_arm_devices_supported() {
        for &id in ARM_DEVICE_IDS {
            let info = classify_device(id);
            assert!(
                info.is_supported(),
                "ARM device 0x{:04X} should be supported",
                id
            );
            assert_eq!(info.firmware, Some(FirmwareType::Am32));
        }
    }
}
