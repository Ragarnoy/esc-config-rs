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
    pub fn session(&self) -> &Session {
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

        self.port
            .write_all(&packet)
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

        self.port
            .write_all(&packet)
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

        self.port
            .write_all(&packet)
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

        self.port
            .write_all(&packet)
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
    #[allow(dead_code)]
    pub fn read_256(&mut self) -> Result<[u8; 256]> {
        let packet = self
            .session
            .read_256_command()
            .map_err(|e| anyhow::anyhow!("protocol error: {e}"))?;

        self.port
            .write_all(&packet)
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

    /// Read 16 bytes from the current address.
    pub fn read_16(&mut self) -> Result<[u8; 16]> {
        let packet = self
            .session
            .read_16_command()
            .map_err(|e| anyhow::anyhow!("protocol error: {e}"))?;

        self.port
            .write_all(&packet)
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

    /// Read 48 bytes from the current address.
    /// Used by Arduino before write operations.
    pub fn read_48(&mut self) -> Result<[u8; 48]> {
        use esc_protocol::packet::build_read_48_packet;

        if !self.session.is_connected() {
            bail!("not connected");
        }

        let packet = build_read_48_packet();
        self.port
            .write_all(&packet)
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
        self.port
            .write_all(&packet)
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

        self.port
            .write_all(&header)
            .context("failed to send write header")?;
        self.port.flush().context("failed to flush write header")?;

        let mut response = [0u8; 1];
        self.read_exact(&mut response)
            .context("failed to read write header response")?;

        if response[0] != 0x30 && response[0] != 0x31 {
            bail!("unexpected write header response: 0x{:02X}", response[0]);
        }

        self.port.write_all(data).context("failed to send data")?;
        let crc = Session::compute_write_crc(data);
        self.port.write_all(&crc).context("failed to send CRC")?;
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

        self.port
            .write_all(&packet)
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

        self.port
            .write_all(&packet)
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

    /// Read the configuration block from the ESC.
    ///
    /// Uses Arduino's read pattern (48 + 128 bytes) to maintain ESC state
    /// that allows subsequent writes.
    pub fn read_config(&mut self) -> Result<ConfigBlock> {
        use esc_protocol::config::CONFIG_ADDRESS;

        // Arduino pattern: read 48 bytes at 0x7C00, then 128 bytes at 0x7C30
        // This leaves the ESC in a state that accepts writes
        self.set_address(CONFIG_ADDRESS)?;
        let first_48 = self.read_48()?;

        self.set_address(0x7C30)?;
        let next_128 = self.read_128()?;

        // Combine into 256-byte config block
        // Bytes 0-47: from first read
        // Bytes 48-175: from second read
        // Bytes 176-255: padding (0xFF = empty flash)
        let mut data = [0xFFu8; 256];
        data[0..48].copy_from_slice(&first_48);
        data[48..176].copy_from_slice(&next_128);

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

    /// Write configuration block to the ESC.
    ///
    /// The relay firmware handles the AM32 write sequence atomically:
    /// 1. Set Address → ACK
    /// 2. Set Buffer → NO ACK (critical!)
    /// 3. Payload (data + CRC) → ACK
    /// 4. Commit → ACK
    ///
    /// Call `read_config()` before this to put the ESC in the correct state.
    pub fn write_config(&mut self, config: &ConfigBlock) -> Result<()> {
        use esc_protocol::config::CONFIG_ADDRESS;
        use esc_protocol::packet::build_set_address_packet;

        // Only first 48 bytes are needed for config
        let config_data = &config.as_bytes()[0..48];

        // Send set_address + write trigger to relay
        // Relay will handle the actual AM32 write sequence
        let set_addr = build_set_address_packet(CONFIG_ADDRESS);
        let write_hdr: [u8; 3] = [0xFE, 0x30, 0x00]; // Trigger for relay

        let mut combined = [0u8; 9];
        combined[0..6].copy_from_slice(&set_addr);
        combined[6..9].copy_from_slice(&write_hdr);

        self.port.write_all(&combined).context("failed to send write command")?;

        // Send data (relay will add proper CRC)
        let crc = Session::compute_write_crc(config_data);
        self.port.write_all(config_data)?;
        self.port.write_all(&crc)?;
        self.port.flush()?;

        // Read responses: set_addr ACK + payload ACK
        let mut response = [0u8; 2];
        self.read_exact(&mut response).context("failed to read write responses")?;

        if response[0] != 0x30 {
            bail!("set_address failed: 0x{:02X}", response[0]);
        }
        if response[1] != 0x30 {
            bail!("write failed: 0x{:02X} (0xC2 = CRC error)", response[1]);
        }

        self.commit()?;
        Ok(())
    }

    // ------------------------------------------------------------------------
    // Internal Helpers
    // ------------------------------------------------------------------------

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
