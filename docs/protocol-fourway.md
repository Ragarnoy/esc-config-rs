# Four-Way Interface Protocol

The Four-Way Interface allows configuring ESCs through a flight controller running Betaflight/INAV.
This is the standard method used by BLHeliSuite and other configurators.

## Connection Flow

```
┌─────────────────────────────────────────────────────────────────┐
│                    Four-Way Connection Flow                      │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  1. Open serial port at 19200 baud                              │
│                                                                  │
│  2. Send MSP_API_VERSION (0x68)                                 │
│     └─> Verify Betaflight responds                              │
│                                                                  │
│  3. Send MSP_SET_4WAY_IF (0xF5)                                 │
│     └─> FC enters passthrough mode                              │
│     └─> Response contains number of motors                      │
│                                                                  │
│  4. Send cmd_DeviceInitFlash (0x37) with motor index            │
│     └─> ESC enters bootloader                                   │
│     └─> Response contains device type (F051/G071/F3)            │
│                                                                  │
│  5. Read/Write operations via cmd_DeviceRead/Write              │
│                                                                  │
│  6. Send cmd_DeviceReset (0x35) to restart ESC                  │
│                                                                  │
│  7. Send cmd_InterfaceExit (0x34) to exit 4-way mode            │
│                                                                  │
│  8. Send MSP_SET_REBOOT (0x44) to reset FC                      │
│     └─> CRITICAL: FC may hold last throttle value otherwise!    │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

---

## Command Packet Structure

```
┌──────┬───────┬──────────┬──────────┬─────────────┬──────────┬──────────┐
│ CMD  │ PARAM │ ADDR_HI  │ ADDR_LO  │ DATA        │ CRC_LO   │ CRC_HI   │
│ u8   │ u8    │ u8       │ u8       │ [u8; N]     │ u8       │ u8       │
└──────┴───────┴──────────┴──────────┴─────────────┴──────────┴──────────┘
```

CRC is CRC-16/IBM over all bytes before CRC.

---

## Commands

| Code | Name | Param | Address | Data | Description |
|------|------|-------|---------|------|-------------|
| 0x30 | InterfaceTestAlive | 0 | - | - | Test if interface alive |
| 0x31 | ProtocolGetVersion | 0 | - | - | Get protocol version |
| 0x32 | InterfaceGetName | 0 | - | - | Get interface name |
| 0x33 | InterfaceGetVersion | 0 | - | - | Get interface version |
| 0x34 | InterfaceExit | 0 | - | - | Exit 4-way mode |
| 0x35 | DeviceReset | motor | - | - | Reset specific ESC |
| 0x37 | DeviceInitFlash | motor | - | - | Connect to ESC (enter bootloader) |
| 0x38 | DeviceEraseAll | 0 | - | - | Erase all flash |
| 0x39 | DevicePageErase | 0 | page | - | Erase page |
| 0x3A | DeviceRead | size | addr | - | Read from address |
| 0x3B | DeviceWrite | size | addr | data | Write to address |
| 0x3C | DeviceC2CK_LOW | 0 | - | - | C2CK low (SiLabs) |
| 0x3D | DeviceReadEEprom | size | addr | - | Read EEPROM |
| 0x3E | DeviceWriteEEprom | size | addr | data | Write EEPROM |
| 0x3F | DeviceReset2 | motor | - | - | Reset device (alias) |

---

## Response Packet Structure

```
┌──────┬──────┬──────────┬──────────┬──────┬─────────────┬──────────┬──────────┐
│ CMD  │ ECHO │ ADDR_HI  │ ADDR_LO  │ ACK  │ DATA        │ CRC_LO   │ CRC_HI   │
│ u8   │ u8   │ u8       │ u8       │ u8   │ [u8; N]     │ u8       │ u8       │
└──────┴──────┴──────────┴──────────┴──────┴─────────────┴──────────┴──────────┘
         │                           │
         │                           └─ ACK at position [len-3]
         └─ Command echo at position [1]
```

---

## ACK Values

| Value | Name | Description |
|-------|------|-------------|
| 0x00 | ACK_OK | Success |
| 0x01 | ACK_I_UNKNOWN_ERROR | Unknown error |
| 0x02 | ACK_I_INVALID_CMD | Invalid command |
| 0x03 | ACK_I_INVALID_CRC | CRC error |
| 0x04 | ACK_I_VERIFY_ERROR | Verification failed |
| 0x05 | ACK_D_GENERAL_ERROR | Device general error |

---

## DeviceInitFlash (0x37) Response

When connecting to an ESC, the response contains device information:

```
Position:  [0]   [1]   [2]   [3]   [4]   [5]   [6]   [7]   [8]   [9]   [10]  [11]
Content:   0x37  ??    ??    ??    DEV   DEV   DIAG  DIAG  DIAG  ACK   CRC   CRC
                                   TYPE  HI    1     2     3           LO    HI
```

**Device Type (byte 4):**
| Value | MCU | Page Size | EEPROM Address | Notes |
|-------|-----|-----------|----------------|-------|
| 0x1F | STM32F051 | 1KB | 0x7C00 | No address division |
| 0x2B | STM32G071 | 2KB | 0x7E00 | Addresses divided by 4! |
| 0x35 | STM32F3xx | 2KB | 0xF800 | No address division |

---

## Rust Implementation

```rust
// crates/esc-protocol/src/fourway.rs

use crate::crc::crc16_ibm;

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum FourWayCommand {
    InterfaceTestAlive = 0x30,
    ProtocolGetVersion = 0x31,
    InterfaceGetName = 0x32,
    InterfaceGetVersion = 0x33,
    InterfaceExit = 0x34,
    DeviceReset = 0x35,
    DeviceInitFlash = 0x37,
    DeviceEraseAll = 0x38,
    DevicePageErase = 0x39,
    DeviceRead = 0x3A,
    DeviceWrite = 0x3B,
    DeviceC2CkLow = 0x3C,
    DeviceReadEeprom = 0x3D,
    DeviceWriteEeprom = 0x3E,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum FourWayAck {
    Ok = 0x00,
    UnknownError = 0x01,
    InvalidCmd = 0x02,
    InvalidCrc = 0x03,
    VerifyError = 0x04,
    GeneralError = 0x05,
}

impl TryFrom<u8> for FourWayAck {
    type Error = u8;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x00 => Ok(Self::Ok),
            0x01 => Ok(Self::UnknownError),
            0x02 => Ok(Self::InvalidCmd),
            0x03 => Ok(Self::InvalidCrc),
            0x04 => Ok(Self::VerifyError),
            0x05 => Ok(Self::GeneralError),
            other => Err(other),
        }
    }
}

/// Build a simple 4-way command (no data)
pub fn build_fourway_command(cmd: FourWayCommand, param: u8) -> Vec<u8> {
    let mut packet = vec![cmd as u8, param, 0x00, 0x00];
    let crc = crc16_ibm(&packet);
    packet.push(crc as u8);
    packet.push((crc >> 8) as u8);
    packet
}

/// Build a 4-way read command
pub fn build_fourway_read(address: u16, size: u8) -> Vec<u8> {
    let mut packet = vec![
        FourWayCommand::DeviceRead as u8,
        size,
        (address >> 8) as u8,
        address as u8,
    ];
    let crc = crc16_ibm(&packet);
    packet.push(crc as u8);
    packet.push((crc >> 8) as u8);
    packet
}

/// Build a 4-way write command
pub fn build_fourway_write(address: u16, data: &[u8]) -> Vec<u8> {
    let mut packet = vec![
        FourWayCommand::DeviceWrite as u8,
        data.len() as u8,
        (address >> 8) as u8,
        address as u8,
    ];
    packet.extend_from_slice(data);
    let crc = crc16_ibm(&packet);
    packet.push(crc as u8);
    packet.push((crc >> 8) as u8);
    packet
}

#[derive(Debug, Clone)]
pub struct FourWayResponse {
    pub command: u8,
    pub ack: FourWayAck,
    pub data: Vec<u8>,
}

/// Parse a 4-way response
pub fn parse_fourway_response(data: &[u8]) -> Option<FourWayResponse> {
    if data.len() < 6 {
        return None;
    }

    // Verify CRC (last 2 bytes)
    let crc_received = u16::from_le_bytes([data[data.len()-2], data[data.len()-1]]);
    let crc_calculated = crc16_ibm(&data[..data.len()-2]);
    if crc_received != crc_calculated {
        return None;
    }

    // ACK is at position len-3
    let ack = FourWayAck::try_from(data[data.len() - 3]).ok()?;

    // Command echo at position 1
    let command = data[1];

    // Data is between header (4 bytes) and ACK/CRC (3 bytes)
    let data_start = 4;
    let data_end = data.len().saturating_sub(3);
    let payload = if data_end > data_start {
        data[data_start..data_end].to_vec()
    } else {
        Vec::new()
    };

    Some(FourWayResponse { command, ack, data: payload })
}
```

---

## Timing Requirements

| Operation | Timeout |
|-----------|---------|
| DeviceInitFlash (connect) | 2000ms |
| Read/Write | 500ms |
| Other commands | 200ms |

---

## C++ Reference

From `widget.cpp`:

```cpp
// Connect to motor
four_way->ack_required = true;
writeData(four_way->makeFourWayCommand(0x37, motor));
m_serial->waitForBytesWritten(200);
while (m_serial->waitForReadyRead(200)) {}
readData();

// Read config
writeData(four_way->makeFourWayReadCommand(buffer_length, eeprom_address));

// Write config
writeData(four_way->makeFourWayWriteCommand(data, size, address));

// Exit and reset FC
writeData(four_way->makeFourWayCommand(0x34, 0x00));  // Exit
send_mspCommand(0x44, 0x00);  // Reset FC - CRITICAL!
```
