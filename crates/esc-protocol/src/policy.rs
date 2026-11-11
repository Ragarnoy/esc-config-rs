//! Which bytes of the AM32 settings structure a host may change.
//!
//! The structure is 192 bytes: boot byte and version bytes (0..5), settings
//! (5..48), melody (48..176), CAN settings (176..184), reserved (184..192). A write
//! replaces all of it (an aligned write erases the page), so every edit is a
//! read-modify-write of the current block. These functions are that merge, shared
//! by every host (the relay CLI, Elle's host tool).

use core::fmt;
use core::ops::Range;

use crate::melody::MELODY_OFFSET;

/// Bytes of the AM32 settings structure.
pub const EEPROM_SIZE: usize = 192;

/// Size of a backup file: 256 bytes read at the EEPROM base.
pub const BACKUP_SIZE: usize = 256;

/// The settings bytes: after boot byte and version bytes, before the melody.
pub const SETTINGS: Range<usize> = 5..MELODY_OFFSET;

/// Bytes a restore takes from the backup: everything after the version bytes.
pub const RESTORED: Range<usize> = 5..EEPROM_SIZE;

/// Offset of the EEPROM layout version.
const LAYOUT_VERSION: usize = 1;

/// Copy the settings bytes of `edited` over `current`.
///
/// Boot byte, version bytes, melody and CAN settings stay as they are in
/// `current`, so an edit made from an older read cannot undo a melody upload,
/// and the firmware does not see a version change.
pub fn merge_settings(current: &mut [u8; EEPROM_SIZE], edited: &[u8; EEPROM_SIZE]) {
    current[SETTINGS].copy_from_slice(&edited[SETTINGS]);
}

/// Copy the melody (128 bytes) over `current`, keeping everything else.
pub fn merge_melody(current: &mut [u8; EEPROM_SIZE], melody: &[u8; 128]) {
    current[MELODY_OFFSET..MELODY_OFFSET + 128].copy_from_slice(melody);
}

/// Why a backup cannot be restored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreError {
    /// Not 256 or 192 bytes.
    Length(usize),
    /// The backup's EEPROM layout version differs from the ESC's.
    LayoutVersion {
        /// Version in the backup.
        backup: u8,
        /// Version on the ESC.
        esc: u8,
    },
}

impl fmt::Display for RestoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Length(n) => write!(
                f,
                "invalid backup: expected {BACKUP_SIZE} or {EEPROM_SIZE} bytes, got {n}"
            ),
            Self::LayoutVersion { backup, esc } => write!(
                f,
                "backup has EEPROM layout version {backup} but the ESC has {esc}; \
                 restore it on matching firmware, or use export/import (TOML)"
            ),
        }
    }
}

/// Merge a backup (256 or 192 bytes) into `current`: bytes 5..192 from the
/// backup, boot and version bytes from the ESC. Refuses a backup with another
/// EEPROM layout version.
pub fn merge_restore(current: &mut [u8; EEPROM_SIZE], backup: &[u8]) -> Result<(), RestoreError> {
    if backup.len() != BACKUP_SIZE && backup.len() != EEPROM_SIZE {
        return Err(RestoreError::Length(backup.len()));
    }
    if backup[LAYOUT_VERSION] != current[LAYOUT_VERSION] {
        return Err(RestoreError::LayoutVersion {
            backup: backup[LAYOUT_VERSION],
            esc: current[LAYOUT_VERSION],
        });
    }
    current[RESTORED].copy_from_slice(&backup[RESTORED]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn esc() -> [u8; EEPROM_SIZE] {
        let mut e = [0u8; EEPROM_SIZE];
        e[..5].copy_from_slice(&[0x01, 4, 14, 2, 19]);
        for (i, b) in e.iter_mut().enumerate().skip(5) {
            *b = i as u8;
        }
        e
    }

    #[test]
    fn settings_merge_keeps_header_melody_and_can() {
        let mut current = esc();
        let mut edited = [0xEE; EEPROM_SIZE];
        edited[0] = 0;
        merge_settings(&mut current, &edited);
        assert_eq!(&current[..5], &[0x01, 4, 14, 2, 19]);
        assert!(current[SETTINGS].iter().all(|&b| b == 0xEE));
        assert!(
            current[48..]
                .iter()
                .enumerate()
                .all(|(i, &b)| b == (i + 48) as u8)
        );
    }

    #[test]
    fn melody_merge_touches_only_the_melody() {
        let mut current = esc();
        merge_melody(&mut current, &[0x42; 128]);
        assert!(current[48..176].iter().all(|&b| b == 0x42));
        assert_eq!(current[47], 47);
        assert_eq!(current[176], 176);
    }

    #[test]
    fn restore_merges_and_checks() {
        let mut current = esc();
        let mut backup = [0xAA; BACKUP_SIZE];
        backup[0] = 0;
        backup[1] = 4;
        merge_restore(&mut current, &backup).unwrap();
        assert_eq!(&current[..5], &[0x01, 4, 14, 2, 19]);
        assert!(current[RESTORED].iter().all(|&b| b == 0xAA));

        backup[1] = 3;
        assert_eq!(
            merge_restore(&mut current, &backup),
            Err(RestoreError::LayoutVersion { backup: 3, esc: 4 })
        );
        assert_eq!(
            merge_restore(&mut current, &[0; 100]),
            Err(RestoreError::Length(100))
        );
        assert!(merge_restore(&mut current, &backup[..EEPROM_SIZE]).is_err());
    }
}
