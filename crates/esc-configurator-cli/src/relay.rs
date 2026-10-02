//! Talking to an ESC through the relay MCU.
//!
//! [`RelayLink`] is an [`esc_session::Link`] over the relay's USB CDC passthrough:
//! the relay forwards each USB write to the ESC wire as one frame and echoes it
//! back (as a one-wire bus would), followed by the ESC's reply. All protocol
//! sequencing and timing is in `esc-session`; [`Relay`] is a blocking facade over
//! it for the CLI commands.

use std::io::{ErrorKind, Read, Write};
use std::time::{Duration as StdDuration, Instant};

use anyhow::{Context, Result};
use embassy_futures::block_on;
use esc_protocol::am32_config::Am32Config;
use esc_protocol::device::DeviceInfo;
use esc_protocol::layout::Layout;
use esc_protocol::melody::MELODY_OFFSET;
use esc_protocol::packet::{BAUD_RATE, ConnectResponse, MAX_BLOCK};
use esc_session::{Duration, EEPROM_SIZE, Esc, Link, Timing};
use serialport::{ClearBuffer, SerialPort};

/// The relay echoes a frame only after the ESC's reply (or its own reply
/// timeout, 500 ms) has passed.
const ECHO_TIMEOUT: StdDuration = StdDuration::from_millis(1500);

/// Added to every reply timeout for USB and relay latency.
const USB_MARGIN: StdDuration = StdDuration::from_millis(100);

/// Settings bytes of the AM32 EEPROM: after boot byte and version bytes (0..5),
/// before the melody (48..176).
pub const SETTINGS: std::ops::Range<usize> = 5..MELODY_OFFSET;

/// [`esc_session::Link`] over the relay's USB passthrough.
pub struct RelayLink {
    port: Box<dyn SerialPort>,
}

impl RelayLink {
    /// Open the relay's serial port.
    pub fn open(device: &str) -> Result<Self> {
        let port = serialport::new(device, BAUD_RATE)
            .timeout(ECHO_TIMEOUT)
            .open()
            .with_context(|| format!("failed to open serial port: {device}"))?;
        port.clear(ClearBuffer::All)
            .context("failed to clear serial buffers")?;
        Ok(Self { port })
    }

    /// Read up to `buf.len()` bytes until `deadline`. Returns the count.
    fn read_until(&mut self, buf: &mut [u8], deadline: Instant) -> std::io::Result<usize> {
        let mut n = 0;
        while n < buf.len() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            self.port.set_timeout(left)?;
            match self.port.read(&mut buf[n..]) {
                Ok(0) => return Err(ErrorKind::UnexpectedEof.into()),
                Ok(k) => n += k,
                Err(e) if e.kind() == ErrorKind::TimedOut => break,
                Err(e) => return Err(e),
            }
        }
        Ok(n)
    }
}

impl Link for RelayLink {
    type Error = std::io::Error;

    async fn send(&mut self, frame: &[u8]) -> std::io::Result<()> {
        self.port.write_all(frame)?;
        self.port.flush()?;
        let mut echo = vec![0u8; frame.len()];
        let n = self.read_until(&mut echo, Instant::now() + ECHO_TIMEOUT)?;
        if n < frame.len() {
            return Err(std::io::Error::new(
                ErrorKind::TimedOut,
                format!("relay echoed {n} of {} bytes", frame.len()),
            ));
        }
        if echo != frame {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                "relay echo does not match the frame sent",
            ));
        }
        Ok(())
    }

    async fn recv(&mut self, buf: &mut [u8], timeout: Duration) -> std::io::Result<usize> {
        let timeout = StdDuration::from_micros(u64::from(timeout.as_micros())) + USB_MARGIN;
        self.read_until(buf, Instant::now() + timeout)
    }

    async fn delay(&mut self, duration: Duration) {
        std::thread::sleep(StdDuration::from_micros(u64::from(duration.as_micros())));
    }
}

/// A connected ESC, blocking API.
///
/// Dropping it leaves the ESC in its bootloader; [`Relay::run`] starts the app.
pub struct Relay {
    esc: Esc<RelayLink>,
}

impl Relay {
    /// Open the relay and connect to the ESC bootloader. Refuses non-AM32 ESCs.
    pub fn connect(device: &str) -> Result<Self> {
        let link = RelayLink::open(device)?;
        let esc = block_on(Esc::connect_with(link, Timing::default()))
            .context("failed to connect to the ESC bootloader")?;
        Ok(Self { esc })
    }

    /// Device classification.
    pub fn device(&self) -> DeviceInfo {
        *self.esc.device()
    }

    /// Flash layout (protocol units).
    pub fn layout(&self) -> Layout {
        *self.esc.layout()
    }

    /// The raw connect reply.
    pub fn connect_reply(&self) -> ConnectResponse {
        *self.esc.connect_reply()
    }

    /// Firmware file name, e.g. `AM32_F4A_4IN1_F421`. Empty if none.
    pub fn read_esc_name(&mut self) -> Result<String> {
        let raw = block_on(self.esc.read_file_name()).context("failed to read the ESC name")?;
        let end = raw
            .iter()
            .position(|&b| b == 0x00 || b == 0xFF)
            .unwrap_or(raw.len());
        Ok(String::from_utf8_lossy(&raw[..end]).into_owned())
    }

    /// Read the settings block (192 bytes; the rest of the 256-byte view is 0xFF).
    pub fn read_settings(&mut self) -> Result<Am32Config> {
        let eeprom = block_on(self.esc.read_eeprom()).context("failed to read settings")?;
        let mut raw = [0xFFu8; 256];
        raw[..EEPROM_SIZE].copy_from_slice(&eeprom);
        Ok(Am32Config::from_bytes(raw))
    }

    /// Write the settings block (first 192 bytes: config, melody, CAN).
    pub fn write_settings(&mut self, config: &Am32Config) -> Result<()> {
        let mut block = [0u8; EEPROM_SIZE];
        block.copy_from_slice(&config.as_bytes()[..EEPROM_SIZE]);
        block_on(self.esc.write_eeprom(&block)).context("failed to write settings")
    }

    /// Write the settings of `edited` (bytes 5..48) over the ESC's current block.
    ///
    /// Boot byte, version bytes, melody and CAN settings stay as they are on the ESC,
    /// so an edit made from an older read cannot undo a melody upload, and the
    /// firmware does not see a version change.
    pub fn update_settings(&mut self, edited: &Am32Config) -> Result<()> {
        let mut current = self.read_settings()?;
        current.as_bytes_mut()[SETTINGS].copy_from_slice(&edited.as_bytes()[SETTINGS]);
        self.write_settings(&current)
    }

    /// Read 256 bytes at the EEPROM base (the backup file format).
    pub fn read_eeprom_256(&mut self) -> Result<[u8; MAX_BLOCK]> {
        let mut raw = [0u8; MAX_BLOCK];
        let base = self.esc.layout().eeprom_start;
        block_on(self.esc.read_at(base, &mut raw)).context("failed to read the EEPROM")?;
        Ok(raw)
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
        let mut config = self.read_settings()?;
        config.as_bytes_mut()[MELODY_OFFSET..MELODY_OFFSET + 128].copy_from_slice(melody);
        self.write_settings(&config)
    }

    /// Flash an app image. See [`Esc::flash_firmware`].
    pub fn flash_firmware(
        &mut self,
        image: &[u8],
        progress: impl FnMut(usize, usize),
    ) -> Result<()> {
        block_on(self.esc.flash_firmware(image, progress)).context("flash failed")
    }

    /// Compare the app region with an image.
    pub fn verify_firmware(
        &mut self,
        image: &[u8],
        progress: impl FnMut(usize, usize),
    ) -> Result<()> {
        block_on(self.esc.verify_firmware(image, progress)).context("verify failed")
    }

    /// Leave the bootloader and start the ESC firmware (if EEPROM byte 0 is 0x01).
    pub fn run(self) -> Result<()> {
        block_on(self.esc.run()).context("failed to send Run")?;
        Ok(())
    }
}

/// List available serial ports.
pub fn list_ports() -> Result<Vec<String>> {
    let ports = serialport::available_ports().context("failed to list serial ports")?;

    Ok(ports.into_iter().map(|p| p.port_name).collect())
}
