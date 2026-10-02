//! Relay MCU communication.
//!
//! This module handles USB serial communication with the relay MCU (RP2350),
//! which bridges between the PC and the ESC bootloader.
//!
//! The relay handles timing-critical operations like the AM32 write sequence
//! atomically to avoid USB latency issues.

use std::io::{Read, Write};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use esc_protocol::bootloader::Session;
use esc_protocol::config::ConfigBlock;
use esc_protocol::packet::BAUD_RATE;
use serialport::SerialPort;

/// Default serial port timeout.
const DEFAULT_TIMEOUT: Duration = Duration::from_millis(500);

/// Response timeout for connect (ESC bootloader may be slow to respond).
const CONNECT_TIMEOUT: Duration = Duration::from_millis(2000);

/// Relay connection wrapping a serial port and protocol session.
pub struct Relay {
    port: Box<dyn SerialPort>,
    session: Session,
}

impl Relay {
    /// Open a connection to the relay MCU.
    pub fn open(device: &str) -> Result<Self> {
        let port = serialport::new(device, BAUD_RATE)
            .timeout(DEFAULT_TIMEOUT)
            .open()
            .with_context(|| format!("failed to open serial port: {device}"))?;

        Ok(Self {
            port,
            session: Session::new(),
        })
    }

    /// Get the underlying protocol session.
    pub const fn session(&self) -> &Session {
        &self.session
    }

    /// Connect to the ESC bootloader.
    ///
    /// Sends the connect handshake and waits for the ESC to respond.
    pub fn connect(&mut self) -> Result<()> {
        // Set longer timeout for connect
        self.port
            .set_timeout(CONNECT_TIMEOUT)
            .context("failed to set timeout")?;

        let packet = self
            .session
            .connect_command()
            .map_err(|e| anyhow::anyhow!("protocol error: {e}"))?;

        self.send(&packet)
            .context("failed to send connect packet")?;

        // Read response: "471" + device_id (2) + diag (3) + ACK = 9 bytes
        let mut response = [0u8; 9];
        self.read_exact(&mut response)
            .context("failed to read connect response")?;

        self.session
            .handle_connect_response(&response)
            .map_err(|e| anyhow::anyhow!("connect failed: {e}"))?;

        // Restore default timeout
        self.port
            .set_timeout(DEFAULT_TIMEOUT)
            .context("failed to reset timeout")?;

        Ok(())
    }

    /// Send keepalive to prevent ESC timeout.
    #[allow(dead_code)]
    pub fn keepalive(&mut self) -> Result<()> {
        let packet = self
            .session
            .keepalive_command()
            .map_err(|e| anyhow::anyhow!("protocol error: {e}"))?;

        self.send(&packet)
            .context("failed to send keepalive")?;

        let mut response = [0u8; 1];
        self.read_exact(&mut response)
            .context("failed to read keepalive response")?;

        self.session
            .handle_keepalive_response(response[0])
            .map_err(|e| anyhow::anyhow!("keepalive failed: {e}"))?;

        Ok(())
    }

    /// Disconnect from the ESC bootloader.
    pub fn disconnect(&mut self) -> Result<()> {
        let packet = self
            .session
            .disconnect_command()
            .map_err(|e| anyhow::anyhow!("protocol error: {e}"))?;

        self.send(&packet)
            .context("failed to send disconnect")?;

        let mut response = [0u8; 1];
        self.read_exact(&mut response)
            .context("failed to read disconnect response")?;

        self.session
            .handle_disconnect_response(response[0])
            .map_err(|e| anyhow::anyhow!("disconnect failed: {e}"))?;

        Ok(())
    }

    /// Set the memory address for subsequent read/write operations.
    pub fn set_address(&mut self, address: u16) -> Result<()> {
        let packet = self
            .session
            .set_address_command(address)
            .map_err(|e| anyhow::anyhow!("protocol error: {e}"))?;

        self.send(&packet)
            .context("failed to send set_address")?;

        let mut response = [0u8; 1];
        self.read_exact(&mut response)
            .context("failed to read set_address response")?;

        self.session
            .handle_set_address_response(response[0], address)
            .map_err(|e| anyhow::anyhow!("set_address failed: {e}"))?;

        Ok(())
    }

    /// Read 256 bytes from the current address.
    ///
    /// Returns the raw data (256 bytes, encrypted for config block).
    pub fn read_256(&mut self) -> Result<[u8; 256]> {
        let packet = self
            .session
            .read_256_command()
            .map_err(|e| anyhow::anyhow!("protocol error: {e}"))?;

        self.send(&packet)
            .context("failed to send read command")?;

        // Response: 256 data + 2 CRC + 1 ACK = 259 bytes
        let mut response = [0u8; 259];
        self.read_exact(&mut response)
            .context("failed to read data")?;

        let data = self
            .session
            .validate_read_response(&response, 256)
            .map_err(|e| anyhow::anyhow!("read failed: {e}"))?;

        let mut result = [0u8; 256];
        result.copy_from_slice(data);
        Ok(result)
    }

    /// Read a chunk from flash memory.
    ///
    /// Reads up to 256 bytes from the specified flash address.
    /// For verification, typically reads in 256-byte chunks.
    pub fn read_flash(&mut self, address: u16, size: usize) -> Result<Vec<u8>> {
        if !self.session.is_connected() {
            bail!("not connected");
        }

        self.set_address(address)?;

        // Read 256 bytes and truncate to requested size
        let data = self.read_256()?;

        Ok(data[..size.min(256)].to_vec())
    }

    /// Read 16 bytes from the current address.
    pub fn read_16(&mut self) -> Result<[u8; 16]> {
        let packet = self
            .session
            .read_16_command()
            .map_err(|e| anyhow::anyhow!("protocol error: {e}"))?;

        self.send(&packet)
            .context("failed to send read command")?;

        // Response: 16 data + 2 CRC + 1 ACK = 19 bytes
        let mut response = [0u8; 19];
        self.read_exact(&mut response)
            .context("failed to read data")?;

        let data = self
            .session
            .validate_read_response(&response, 16)
            .map_err(|e| anyhow::anyhow!("read failed: {e}"))?;

        let mut result = [0u8; 16];
        result.copy_from_slice(data);
        Ok(result)
    }

    /// Read 32 bytes from the current address.
    pub fn read_32(&mut self) -> Result<[u8; 32]> {
        use esc_protocol::packet::build_read_32_packet;

        if !self.session.is_connected() {
            bail!("not connected");
        }

        let packet = build_read_32_packet();
        self.send(&packet)
            .context("failed to send read command")?;

        // Response: 32 data + 2 CRC + 1 ACK = 35 bytes
        let mut response = [0u8; 35];
        self.read_exact(&mut response)
            .context("failed to read data")?;

        // Validate: check ACK at end
        if response[34] != 0x30 {
            bail!("read_32 failed: expected ACK 0x30, got 0x{:02X}", response[34]);
        }

        let mut result = [0u8; 32];
        result.copy_from_slice(&response[0..32]);
        Ok(result)
    }

    /// Read 48 bytes from the current address.
    /// Used by Arduino before write operations.
    pub fn read_48(&mut self) -> Result<[u8; 48]> {
        use esc_protocol::packet::build_read_48_packet;

        if !self.session.is_connected() {
            bail!("not connected");
        }

        let packet = build_read_48_packet();
        self.send(&packet)
            .context("failed to send read command")?;

        // Response: 48 data + 2 CRC + 1 ACK = 51 bytes
        let mut response = [0u8; 51];
        self.read_exact(&mut response)
            .context("failed to read data")?;

        // Validate: check ACK at end
        if response[50] != 0x30 {
            bail!("read_48 failed: expected ACK 0x30, got 0x{:02X}", response[50]);
        }

        let mut result = [0u8; 48];
        result.copy_from_slice(&response[0..48]);
        Ok(result)
    }

    /// Read 128 bytes from the current address.
    /// Used by Arduino before write operations.
    pub fn read_128(&mut self) -> Result<[u8; 128]> {
        use esc_protocol::packet::build_read_128_packet;

        if !self.session.is_connected() {
            bail!("not connected");
        }

        let packet = build_read_128_packet();
        self.send(&packet)
            .context("failed to send read command")?;

        // Response: 128 data + 2 CRC + 1 ACK = 131 bytes
        let mut response = [0u8; 131];
        self.read_exact(&mut response)
            .context("failed to read data")?;

        // Validate: check ACK at end
        if response[130] != 0x30 {
            bail!("read_128 failed: expected ACK 0x30, got 0x{:02X}", response[130]);
        }

        let mut result = [0u8; 128];
        result.copy_from_slice(&response[0..128]);
        Ok(result)
    }

    /// Write 256 bytes to the current address (legacy method).
    ///
    /// Note: For AM32 config writes, use `write_config()` instead which
    /// sends the correct atomic sequence through the relay firmware.
    #[allow(dead_code)]
    pub fn write_256(&mut self, data: &[u8; 256]) -> Result<()> {
        if !self.session.is_connected() {
            bail!("not connected");
        }

        // AM32 write header
        let header: [u8; 3] = [0xFE, 0x00, 0x00];

        self.send(&header)
            .context("failed to send write header")?;
        self.port.flush().context("failed to flush write header")?;

        let mut response = [0u8; 1];
        self.read_exact(&mut response)
            .context("failed to read write header response")?;

        if response[0] != 0x30 && response[0] != 0x31 {
            bail!("unexpected write header response: 0x{:02X}", response[0]);
        }

        self.send(data).context("failed to send data")?;
        let crc = Session::compute_write_crc(data);
        self.send(&crc).context("failed to send CRC")?;
        self.port.flush().context("failed to flush write data")?;

        self.read_exact(&mut response)
            .context("failed to read write data response")?;

        self.session
            .handle_ack_response(response[0])
            .map_err(|e| anyhow::anyhow!("write failed: {e}"))?;

        Ok(())
    }

    /// Commit pending write to flash.
    pub fn commit(&mut self) -> Result<()> {
        let packet = self
            .session
            .commit_command()
            .map_err(|e| anyhow::anyhow!("protocol error: {e}"))?;

        self.send(&packet)
            .context("failed to send commit")?;

        self.port.flush().context("failed to flush commit")?;

        let mut response = [0u8; 1];
        self.read_exact(&mut response)
            .context("failed to read commit response")?;

        self.session
            .handle_ack_response(response[0])
            .map_err(|e| anyhow::anyhow!("commit failed: {e}"))?;

        Ok(())
    }

    /// Erase the flash page at the current address.
    ///
    /// Must set address first. Required before writing to AM32 ESCs.
    #[allow(dead_code)]
    pub fn erase_page(&mut self) -> Result<()> {
        let packet = self
            .session
            .erase_page_command()
            .map_err(|e| anyhow::anyhow!("protocol error: {e}"))?;

        self.send(&packet)
            .context("failed to send erase")?;

        self.port.flush().context("failed to flush erase")?;

        let mut response = [0u8; 1];
        self.read_exact(&mut response)
            .context("failed to read erase response")?;

        self.session
            .handle_ack_response(response[0])
            .map_err(|e| anyhow::anyhow!("erase failed: {e}"))?;

        Ok(())
    }

    // ------------------------------------------------------------------------
    // High-level Operations
    // ------------------------------------------------------------------------

    /// Get the EEPROM/config base address for the connected device.
    ///
    /// Returns the MCU-specific eeprom offset (e.g. 0x7C00, 0x7E00, 0xF800).
    fn eeprom_offset(&self) -> Result<u16> {
        let device = self.session.device()
            .context("not connected: cannot determine eeprom offset")?;
        Ok(device.eeprom_offset())
    }

    /// Read the configuration block from the ESC.
    pub fn read_config(&mut self) -> Result<ConfigBlock> {
        let eeprom = self.eeprom_offset()?;
        self.set_address(eeprom)?;
        let data = self.read_256()?;

        Ok(ConfigBlock::from_array(data))
    }

    /// Read the device ID block from the ESC.
    pub fn read_device_id(&mut self) -> Result<[u8; 16]> {
        use esc_protocol::config::DEVICE_ID_ADDRESS;

        self.set_address(DEVICE_ID_ADDRESS)?;
        self.read_16()
    }

    /// Read the timestamp block from the ESC.
    #[allow(dead_code)]
    pub fn read_timestamp(&mut self) -> Result<[u8; 16]> {
        use esc_protocol::config::TIMESTAMP_ADDRESS;

        self.set_address(TIMESTAMP_ADDRESS)?;
        self.read_16()
    }

    /// Read EEPROM (first 48 bytes of config block).
    ///
    /// Used for safety bit handling during firmware flashing.
    pub fn read_eeprom(&mut self) -> Result<[u8; 48]> {
        let eeprom = self.eeprom_offset()?;
        self.set_address(eeprom)?;
        self.read_48()
    }

    /// Write EEPROM (first 48 bytes of config block).
    ///
    /// Used for safety bit handling during firmware flashing.
    pub fn write_eeprom(&mut self, data: &[u8; 48]) -> Result<()> {
        // Build a full 256-byte config block with the 48 bytes
        let mut config_data = [0xFFu8; 256];
        config_data[0..48].copy_from_slice(data);
        let block = ConfigBlock::from_array(config_data);
        self.write_config(&block)
    }

    /// Read the ESC firmware file name (32 bytes before eeprom offset).
    ///
    /// Returns the name as a string (e.g. "AM32_F4A_4IN1_F421").
    /// The MCU type can be extracted as the last segment after '_'.
    pub fn read_esc_name(&mut self) -> Result<String> {
        if !self.session.is_connected() {
            bail!("not connected");
        }

        let eeprom = self.eeprom_offset()?;
        let name_addr = eeprom - 32;
        self.set_address(name_addr)?;
        let data = self.read_32()?;

        // Find null terminator or 0xFF (empty flash)
        let end = data.iter()
            .position(|&b| b == 0x00 || b == 0xFF)
            .unwrap_or(data.len());

        let name = core::str::from_utf8(&data[..end])
            .unwrap_or("")
            .to_string();
        Ok(name)
    }

    /// Read melody data from ESC (128 bytes at config+48).
    pub fn read_melody(&mut self) -> Result<[u8; 128]> {
        use esc_protocol::melody::MELODY_OFFSET;

        if !self.session.is_connected() {
            bail!("not connected");
        }

        let eeprom = self.eeprom_offset()?;
        let melody_addr = eeprom + MELODY_OFFSET as u16;
        self.set_address(melody_addr)?;
        self.read_128()
    }

    /// Write melody data to ESC (128 bytes at config+48).
    ///
    /// Uses read-modify-write of the full 256-byte config block to avoid
    /// erasing the config settings when the flash page is programmed.
    pub fn write_melody(&mut self, data: &[u8; 128]) -> Result<()> {
        use esc_protocol::melody::MELODY_OFFSET;

        if !self.session.is_connected() {
            bail!("not connected");
        }

        // Read the full 256-byte config block so we preserve config settings
        let eeprom = self.eeprom_offset()?;
        self.set_address(eeprom)?;
        let mut block = self.read_256()?;

        // Overlay melody data at offset 48
        block[MELODY_OFFSET..MELODY_OFFSET + 128].copy_from_slice(data);

        // Write the full 256-byte block back
        self.write_eeprom_block(&block)?;
        Ok(())
    }

    /// Write configuration block to the ESC.
    ///
    /// Uses read-modify-write of the full 256-byte config block to avoid
    /// erasing the melody data when the flash page is programmed.
    ///
    /// Call `read_config()` before this to put the ESC in the correct state.
    pub fn write_config(&mut self, config: &ConfigBlock) -> Result<()> {
        let eeprom = self.eeprom_offset()?;

        // Read the full 256-byte config block so we preserve melody data
        self.set_address(eeprom)?;
        let mut block = self.read_256()?;

        // Overlay config data (first 48 bytes only)
        block[0..48].copy_from_slice(&config.as_bytes()[0..48]);

        // Write the full 256-byte block back
        self.write_eeprom_block(&block)?;
        Ok(())
    }

    /// Write config+melody block to the eeprom base address.
    ///
    /// AM32 flash writes erase the whole page, so we must write config and
    /// melody together as a single contiguous block (matching am32.ca which
    /// writes 184 bytes). We write 176 bytes = 48 config + 128 melody.
    ///
    /// The relay firmware handles the AM32 write sequence atomically:
    /// 1. Set Address → ACK
    /// 2. Set Buffer → NO ACK
    /// 3. Payload (data + CRC) → ACK
    /// 4. Commit → ACK
    fn write_eeprom_block(&mut self, block: &[u8; 256]) -> Result<()> {
        use esc_protocol::crc::crc16_ibm_bytes;
        use esc_protocol::melody::MELODY_OFFSET;
        use esc_protocol::packet::build_set_address_packet;

        let eeprom = self.eeprom_offset()?;
        let set_addr = build_set_address_packet(eeprom);

        // Write config (48) + melody (128) = 176 bytes at eeprom base
        let write_size: usize = MELODY_OFFSET + 128; // 176
        let payload = &block[..write_size];
        let size = write_size as u8; // 176

        let set_buffer_cmd: [u8; 4] = [0xFE, 0x00, 0x00, size];
        let set_buffer_crc = crc16_ibm_bytes(&set_buffer_cmd);

        // Combine: set_addr (6) + set_buffer (6) + data (176) + data_crc (2) = 190 bytes
        let combined_len = 6 + 6 + write_size + 2; // 190
        let mut combined = [0u8; 190];
        combined[0..6].copy_from_slice(&set_addr);
        combined[6..10].copy_from_slice(&set_buffer_cmd);
        combined[10..12].copy_from_slice(&set_buffer_crc);
        combined[12..12 + write_size].copy_from_slice(payload);
        let data_crc = crc16_ibm_bytes(payload);
        combined[12 + write_size..combined_len].copy_from_slice(&data_crc);

        self.send(&combined).context("failed to send eeprom write command")?;
        self.port.flush()?;

        // Read responses: set_addr ACK + payload ACK
        let mut response = [0u8; 2];
        self.read_exact(&mut response).context("failed to read write responses")?;

        if response[0] != 0x30 {
            bail!("set_address failed: 0x{:02X}", response[0]);
        }
        if response[1] != 0x30 {
            bail!("eeprom write failed: 0x{:02X} (0xC2 = CRC error)", response[1]);
        }

        self.commit()?;
        Ok(())
    }

    // ------------------------------------------------------------------------
    // Firmware Flashing Operations
    // ------------------------------------------------------------------------

    /// Write a chunk to flash memory (for firmware flashing).
    ///
    /// The write sequence for AM32 (matching C++ ESC Config Tool):
    /// 1. Set Address → ACK
    /// 2. Set Buffer Size → NO ACK (just wait)
    /// 3. Send Data Buffer → ACK
    /// 4. Commit Flash → ACK
    ///
    /// Each command is sent separately through relay passthrough mode.
    ///
    /// # Arguments
    /// * `address` - Flash address to write to
    /// * `data` - Data chunk to write (typically 128 bytes)
    pub fn write_chunk(&mut self, address: u16, data: &[u8]) -> Result<()> {
        use esc_protocol::crc::crc16_ibm_bytes;

        if !self.session.is_connected() {
            bail!("not connected");
        }

        // 1. Set address → ACK
        self.set_address(address)?;

        // 2. Set buffer size (NO ACK expected - AM32 quirk)
        // Format: [0xFE] [0x00] [0x00] [SIZE] [CRC_LO] [CRC_HI]
        let size = data.len() as u8;
        let mut size_pkt = [0u8; 6];
        size_pkt[0..4].copy_from_slice(&[0xFE, 0x00, 0x00, size]);
        let size_crc = crc16_ibm_bytes(&size_pkt[0..4]);
        size_pkt[4..6].copy_from_slice(&size_crc);

        // Send and drain echo (relay echoes, no ESC ACK expected)
        self.send(&size_pkt)?;
        self.port.flush()?;

        // Give ESC time to process SetBuffer before sending data
        std::thread::sleep(Duration::from_millis(30));

        // 3. Send data buffer with CRC → ACK
        let data_crc = crc16_ibm_bytes(data);
        let mut data_pkt = vec![0u8; data.len() + 2];
        data_pkt[..data.len()].copy_from_slice(data);
        data_pkt[data.len()..].copy_from_slice(&data_crc);
        self.send(&data_pkt)?;
        self.port.flush()?;

        // Wait for ACK (longer timeout for flash operations)
        self.port.set_timeout(Duration::from_millis(250))?;
        let mut response = [0u8; 1];
        self.read_exact(&mut response)
            .context("timeout waiting for data ACK")?;

        if response[0] != 0x30 {
            self.port.set_timeout(DEFAULT_TIMEOUT)?;
            bail!("data write failed: ACK=0x{:02X} (expected 0x30)", response[0]);
        }

        // 4. Commit flash → ACK
        self.port.set_timeout(Duration::from_millis(100))?;
        self.commit()?;

        self.port.set_timeout(DEFAULT_TIMEOUT)?;
        Ok(())
    }

    // ------------------------------------------------------------------------
    // Internal Helpers
    // ------------------------------------------------------------------------

    /// Send data and drain the one-wire echo from the relay.
    ///
    /// The relay firmware echoes all transmitted bytes (mimicking a real
    /// one-wire bus where TX appears on RX). This method sends the data,
    /// then reads and discards the echoed bytes.
    fn send(&mut self, data: &[u8]) -> Result<()> {
        self.port.write_all(data).context("failed to send")?;
        let mut echo = vec![0u8; data.len()];
        self.read_exact(&mut echo).context("failed to read echo")?;
        Ok(())
    }

    /// Read exact number of bytes with retry on partial reads.
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        let mut offset = 0;
        while offset < buf.len() {
            match self.port.read(&mut buf[offset..]) {
                Ok(0) => bail!("unexpected EOF"),
                Ok(n) => offset += n,
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                    bail!("timeout waiting for response")
                }
                Err(e) => return Err(e).context("read error"),
            }
        }
        Ok(())
    }
}

/// List available serial ports.
pub fn list_ports() -> Result<Vec<String>> {
    let ports = serialport::available_ports().context("failed to list serial ports")?;

    Ok(ports.into_iter().map(|p| p.port_name).collect())
}
