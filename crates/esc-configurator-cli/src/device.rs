//! ESC operations as the CLI sees them, independent of the transport.
//!
//! [`EscDevice`] is one method per ICD operation (`esc-config-icd`); the relay
//! implements it over USB, tests implement it in memory. [`Session`] holds the
//! host-side policy on top: which EEPROM bytes an edit may touch, how a backup is
//! merged on restore, how a firmware image is cut into chunks.

use anyhow::{Context, Result, bail};
use esc_config_icd::{EEPROM_SIZE, EscInfo, MAX_BLOCK};
use esc_protocol::am32_config::Am32Config;
use esc_protocol::device::{DeviceInfo, classify_device};
use esc_protocol::layout::Layout;
use esc_protocol::melody::MELODY_OFFSET;

use esc_protocol::policy;

/// One ESC operation per method; each runs to completion on the server.
pub trait EscDevice {
    /// Connect to the ESC bootloader (replaces any earlier connection).
    fn connect(&mut self) -> Result<EscInfo>;
    /// Read the 192-byte settings structure.
    fn read_eeprom(&mut self) -> Result<[u8; EEPROM_SIZE]>;
    /// Write the 192-byte settings structure.
    fn write_eeprom(&mut self, data: &[u8; EEPROM_SIZE]) -> Result<()>;
    /// Read the 32-byte firmware file name.
    fn read_file_name(&mut self) -> Result<Vec<u8>>;
    /// Raw read at a protocol address (1..=256 bytes).
    fn read(&mut self, address: u16, len: u16) -> Result<Vec<u8>>;
    /// Clear the boot byte before flashing.
    fn flash_begin(&mut self) -> Result<()>;
    /// Write and verify one chunk (≤ 256 bytes) at an app offset.
    fn flash_chunk(&mut self, offset: u32, data: &[u8]) -> Result<()>;
    /// Set the boot byte after flashing.
    fn flash_finish(&mut self) -> Result<()>;
    /// Read part of the app region (1..=256 bytes).
    fn read_firmware(&mut self, offset: u32, len: u16) -> Result<Vec<u8>>;
    /// Leave the bootloader and start the app.
    fn run(&mut self) -> Result<()>;
}

/// A connected ESC.
///
/// Dropping it leaves the ESC in its bootloader; [`Session::run`] starts the app.
pub struct Session<D: EscDevice> {
    device: D,
    info: EscInfo,
}

impl<D: EscDevice> Session<D> {
    /// Connect through `device`.
    pub fn new(mut device: D) -> Result<Self> {
        let info = device
            .connect()
            .context("failed to connect to the ESC bootloader")?;
        Ok(Self { device, info })
    }

    /// What the ESC reported at connect.
    pub const fn info(&self) -> &EscInfo {
        &self.info
    }

    /// Device classification.
    pub fn device(&self) -> DeviceInfo {
        classify_device(self.info.device_id)
    }

    /// Flash layout (protocol units).
    pub const fn layout(&self) -> Layout {
        let l = self.info.layout;
        Layout {
            address_shift: l.address_shift,
            firmware_start: l.firmware_start,
            filename_start: l.filename_start,
            eeprom_start: l.eeprom_start,
        }
    }

    /// Firmware file name, e.g. `AM32_F4A_4IN1_F421`. Empty if none.
    pub fn read_esc_name(&mut self) -> Result<String> {
        let raw = self
            .device
            .read_file_name()
            .context("failed to read the ESC name")?;
        let end = raw
            .iter()
            .position(|&b| b == 0x00 || b == 0xFF)
            .unwrap_or(raw.len());
        Ok(String::from_utf8_lossy(&raw[..end]).into_owned())
    }

    /// Read the settings block (192 bytes; the rest of the 256-byte view is 0xFF).
    pub fn read_settings(&mut self) -> Result<Am32Config> {
        let eeprom = self
            .device
            .read_eeprom()
            .context("failed to read settings")?;
        let mut raw = [0xFFu8; 256];
        raw[..EEPROM_SIZE].copy_from_slice(&eeprom);
        Ok(Am32Config::from_bytes(raw))
    }

    /// Write the settings block (first 192 bytes: config, melody, CAN).
    pub fn write_settings(&mut self, config: &Am32Config) -> Result<()> {
        let mut block = [0u8; EEPROM_SIZE];
        block.copy_from_slice(&config.as_bytes()[..EEPROM_SIZE]);
        self.device
            .write_eeprom(&block)
            .context("failed to write settings")
    }

    /// Write the settings of `edited` (bytes 5..48) over the ESC's current block.
    ///
    /// Boot byte, version bytes, melody and CAN settings stay as they are on the ESC,
    /// so an edit made from an older read cannot undo a melody upload, and the
    /// firmware does not see a version change.
    pub fn update_settings(&mut self, edited: &Am32Config) -> Result<()> {
        let mut block = self
            .device
            .read_eeprom()
            .context("failed to read settings")?;
        let mut edited_block = [0u8; EEPROM_SIZE];
        edited_block.copy_from_slice(&edited.as_bytes()[..EEPROM_SIZE]);
        policy::merge_settings(&mut block, &edited_block);
        self.device
            .write_eeprom(&block)
            .context("failed to write settings")
    }

    /// Restore a backup (256 or 192 bytes): bytes 5..192 from the backup, boot and
    /// version bytes from the ESC. Refuses a backup with another EEPROM layout
    /// version.
    pub fn restore(&mut self, backup: &[u8]) -> Result<()> {
        let mut block = self
            .device
            .read_eeprom()
            .context("failed to read settings")?;
        policy::merge_restore(&mut block, backup).map_err(|e| anyhow::anyhow!("{e}"))?;
        self.device
            .write_eeprom(&block)
            .context("failed to write settings")
    }

    /// Read 256 bytes at the EEPROM base (the backup file format).
    pub fn read_eeprom_256(&mut self) -> Result<Vec<u8>> {
        #[allow(clippy::cast_possible_truncation)] // 256
        self.device
            .read(self.info.layout.eeprom_start, MAX_BLOCK as u16)
            .context("failed to read the EEPROM")
    }

    /// Read the 128-byte melody (settings bytes 48..176).
    pub fn read_melody(&mut self) -> Result<[u8; 128]> {
        let config = self.read_settings()?;
        let mut melody = [0u8; 128];
        melody.copy_from_slice(&config.as_bytes()[MELODY_OFFSET..MELODY_OFFSET + 128]);
        Ok(melody)
    }

    /// Replace the melody, keeping every other setting (read-modify-write).
    pub fn write_melody(&mut self, melody: &[u8; 128]) -> Result<()> {
        let mut block = self
            .device
            .read_eeprom()
            .context("failed to read settings")?;
        policy::merge_melody(&mut block, melody);
        self.device
            .write_eeprom(&block)
            .context("failed to write settings")
    }

    /// Flash an app image: clear the boot byte, write 256-byte chunks in order
    /// (each read back by the relay), set the boot byte. On error the boot byte
    /// stays cleared and the ESC stays in its bootloader.
    pub fn flash_firmware(
        &mut self,
        image: &[u8],
        mut progress: impl FnMut(usize, usize),
    ) -> Result<()> {
        self.check_image(image)?;
        self.device.flash_begin().context("flash begin failed")?;
        for (i, chunk) in image.chunks(MAX_BLOCK).enumerate() {
            let offset = i * MAX_BLOCK;
            self.device
                .flash_chunk(u32::try_from(offset)?, chunk)
                .with_context(|| format!("flash failed at app offset 0x{offset:05X}"))?;
            progress(offset + chunk.len(), image.len());
        }
        self.device.flash_finish().context("flash finish failed")
    }

    /// Compare the app region with an image.
    pub fn verify_firmware(
        &mut self,
        image: &[u8],
        mut progress: impl FnMut(usize, usize),
    ) -> Result<()> {
        self.check_image(image)?;
        for (i, chunk) in image.chunks(MAX_BLOCK).enumerate() {
            let offset = i * MAX_BLOCK;
            #[allow(clippy::cast_possible_truncation)] // ≤ 256
            let actual = self
                .device
                .read_firmware(u32::try_from(offset)?, chunk.len() as u16)
                .with_context(|| format!("read failed at app offset 0x{offset:05X}"))?;
            if actual != chunk {
                bail!("verify failed at app offset 0x{offset:05X}");
            }
            progress(offset + chunk.len(), image.len());
        }
        Ok(())
    }

    /// Leave the bootloader and start the ESC firmware (if EEPROM byte 0 is 0x01).
    pub fn run(mut self) -> Result<()> {
        self.device.run().context("failed to send Run")
    }

    fn check_image(&self, image: &[u8]) -> Result<()> {
        let max = self.info.max_firmware_size as usize;
        if image.is_empty() {
            bail!("firmware image is empty");
        }
        if image.len() > max {
            bail!("firmware image is {} bytes, only {max} fit", image.len());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use esc_config_icd::LayoutInfo;

    /// An ESC in memory: a 192-byte EEPROM and an app region.
    struct MockDevice {
        eeprom: [u8; EEPROM_SIZE],
        app: Vec<u8>,
        fail_chunk_at: Option<u32>,
        runs: u32,
    }

    impl MockDevice {
        fn new() -> Self {
            let mut eeprom = [0u8; EEPROM_SIZE];
            eeprom[0] = 0x01;
            eeprom[1] = 4;
            eeprom[2] = 14;
            eeprom[3] = 2;
            eeprom[4] = 19;
            for (i, b) in eeprom.iter_mut().enumerate().skip(5) {
                *b = i as u8;
            }
            eeprom[176..184].copy_from_slice(&[7, 1, 1, 50, 0, 0, 0, 1]);
            Self {
                eeprom,
                app: vec![0xFF; 0x6C00],
                fail_chunk_at: None,
                runs: 0,
            }
        }
    }

    impl EscDevice for MockDevice {
        fn connect(&mut self) -> Result<EscInfo> {
            Ok(EscInfo {
                device_id: 0x1F06,
                pin_code: 0x06,
                flash_code: 0x1F,
                protocol_version: 3,
                layout: LayoutInfo {
                    address_shift: 0,
                    firmware_start: 0x1000,
                    filename_start: 0x7BE0,
                    eeprom_start: 0x7C00,
                },
                max_firmware_size: self.app.len() as u32,
            })
        }
        fn read_eeprom(&mut self) -> Result<[u8; EEPROM_SIZE]> {
            Ok(self.eeprom)
        }
        fn write_eeprom(&mut self, data: &[u8; EEPROM_SIZE]) -> Result<()> {
            self.eeprom = *data;
            Ok(())
        }
        fn read_file_name(&mut self) -> Result<Vec<u8>> {
            let mut name = b"AM32_TEST_F051\0".to_vec();
            name.resize(32, 0xFF);
            Ok(name)
        }
        fn read(&mut self, _address: u16, len: u16) -> Result<Vec<u8>> {
            let mut v = self.eeprom.to_vec();
            v.resize(usize::from(len), 0xFF);
            Ok(v)
        }
        fn flash_begin(&mut self) -> Result<()> {
            self.eeprom[0] = 0;
            Ok(())
        }
        fn flash_chunk(&mut self, offset: u32, data: &[u8]) -> Result<()> {
            if self.fail_chunk_at == Some(offset) {
                bail!("Nack(0xC1)");
            }
            let o = offset as usize;
            self.app[o..o + data.len()].copy_from_slice(data);
            Ok(())
        }
        fn flash_finish(&mut self) -> Result<()> {
            self.eeprom[0] = 1;
            Ok(())
        }
        fn read_firmware(&mut self, offset: u32, len: u16) -> Result<Vec<u8>> {
            let o = offset as usize;
            Ok(self.app[o..o + usize::from(len)].to_vec())
        }
        fn run(&mut self) -> Result<()> {
            self.runs += 1;
            Ok(())
        }
    }

    fn session() -> Session<MockDevice> {
        Session::new(MockDevice::new()).unwrap()
    }

    #[test]
    fn settings_edit_keeps_everything_else() {
        let mut s = session();
        let mut edited = s.read_settings().unwrap();
        // Meanwhile the melody changes on the ESC (melody menu in the same session).
        s.write_melody(&[0x42; 128]).unwrap();
        edited.as_bytes_mut()[0] = 0x00; // stale/odd header bytes must not leak
        edited.as_bytes_mut()[3] = 9;
        edited.set_beep_volume(3);
        s.update_settings(&edited).unwrap();

        let e = s.device.eeprom;
        assert_eq!(e[0], 0x01);
        assert_eq!(e[3], 2);
        assert_eq!(e[30], 3, "beep volume written");
        assert!(e[48..176].iter().all(|&b| b == 0x42), "melody kept");
        assert_eq!(&e[176..184], &[7, 1, 1, 50, 0, 0, 0, 1], "CAN kept");
    }

    #[test]
    fn restore_merges_and_checks_the_version() {
        let mut s = session();
        let mut backup = vec![0xAAu8; 256];
        backup[0] = 0x00;
        backup[1] = 4;
        s.restore(&backup).unwrap();
        let e = s.device.eeprom;
        assert_eq!(
            &e[..5],
            &[0x01, 4, 14, 2, 19],
            "boot and version bytes kept"
        );
        assert!(
            e[5..].iter().all(|&b| b == 0xAA),
            "settings, melody, CAN restored"
        );

        backup[1] = 3;
        let err = s.restore(&backup).unwrap_err().to_string();
        assert!(err.contains("layout version 3"), "{err}");
        assert!(s.restore(&[0; 100]).is_err());
    }

    #[test]
    fn flash_writes_chunks_and_sets_the_boot_byte() {
        let mut s = session();
        let image: Vec<u8> = (0..1000u32).map(|i| i as u8).collect();
        let mut calls = Vec::new();
        s.flash_firmware(&image, |done, total| calls.push((done, total)))
            .unwrap();
        assert_eq!(calls.last(), Some(&(1000, 1000)));
        assert_eq!(calls.len(), 4);
        assert_eq!(&s.device.app[..1000], image.as_slice());
        assert_eq!(s.device.eeprom[0], 0x01);
        s.verify_firmware(&image, |_, _| {}).unwrap();
    }

    #[test]
    fn failed_flash_leaves_the_boot_byte_cleared() {
        let mut s = session();
        s.device.fail_chunk_at = Some(512);
        let err = s.flash_firmware(&[0x11; 1024], |_, _| {}).unwrap_err();
        assert!(format!("{err:#}").contains("0x00200"), "{err:#}");
        assert_eq!(s.device.eeprom[0], 0x00);
    }

    #[test]
    fn verify_reports_the_offset() {
        let mut s = session();
        s.flash_firmware(&[0x22; 600], |_, _| {}).unwrap();
        let mut other = vec![0x22; 600];
        other[300] = 0;
        let err = s.verify_firmware(&other, |_, _| {}).unwrap_err();
        assert!(err.to_string().contains("0x00100"), "{err}");
    }

    #[test]
    fn oversized_or_empty_images_are_refused() {
        let mut s = session();
        let max = s.info().max_firmware_size as usize;
        assert!(s.flash_firmware(&vec![0; max + 1], |_, _| {}).is_err());
        assert!(s.flash_firmware(&[], |_, _| {}).is_err());
        assert_eq!(s.device.eeprom[0], 0x01, "refused before touching the ESC");
    }

    #[test]
    fn name_and_run() {
        let mut s = session();
        assert_eq!(s.read_esc_name().unwrap(), "AM32_TEST_F051");
        assert_eq!(s.layout().firmware_byte_offset(), 0x1000);
        s.run().unwrap();
    }
}
