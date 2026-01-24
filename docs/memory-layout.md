# MCU Types and Memory Layout

Different AM32 ESCs use different STM32 MCUs with varying flash sizes and memory layouts.

---

## MCU Types

| Device ID | MCU | Flash | Page Size | EEPROM Address | Address Divider |
|-----------|-----|-------|-----------|----------------|-----------------|
| 0x1F | STM32F051 | 32KB | 1KB | 0x7C00 | None |
| 0x2B | STM32G071 | 128KB | 2KB | 0x7E00 | **Divide by 4** |
| 0x35 | STM32F3xx | 64KB | 2KB | 0xF800 | None |

---

## Memory Maps

### STM32F051 (32KB Flash, 1KB pages)

```
┌──────────────────────────────────────────────────────────────┐
│                    F051 Memory Map (32KB)                    │
├──────────────────────────────────────────────────────────────┤
│                                                              │
│  0x0000 ┌─────────────────────────────────────────────────┐  │
│         │                                                 │  │
│         │              Bootloader                         │  │
│         │                 4KB                             │  │
│         │                                                 │  │
│  0x1000 ├─────────────────────────────────────────────────┤  │
│         │                                                 │  │
│         │              Firmware                           │  │
│         │                27KB                             │  │
│         │                                                 │  │
│  0x7C00 ├─────────────────────────────────────────────────┤  │
│         │  Config (48B) │ Melody (128B) │ Reserved (80B)  │  │
│  0x7CFF └─────────────────────────────────────────────────┘  │
│                                                              │
└──────────────────────────────────────────────────────────────┘

Protocol addresses = Real addresses (no translation needed)
```

### STM32G071 (128KB Flash, 2KB pages)

```
┌──────────────────────────────────────────────────────────────┐
│                    G071 Memory Map (128KB)                   │
│                  ⚠️  ADDRESSES DIVIDED BY 4 ⚠️                │
├──────────────────────────────────────────────────────────────┤
│                                                              │
│  Real      Proto                                             │
│  0x00000   0x0000 ┌───────────────────────────────────────┐  │
│                   │                                       │  │
│                   │            Bootloader                 │  │
│                   │               16KB                    │  │
│                   │                                       │  │
│  0x04000   0x1000 ├───────────────────────────────────────┤  │
│                   │                                       │  │
│                   │            Firmware                   │  │
│                   │              110KB                    │  │
│                   │                                       │  │
│  0x1F800   0x7E00 ├───────────────────────────────────────┤  │
│                   │  Config (48B) │ Melody │ Reserved     │  │
│  0x1F8FF   0x7E3F └───────────────────────────────────────┘  │
│                                                              │
└──────────────────────────────────────────────────────────────┘

⚠️  CRITICAL: Protocol address = Real address / 4
    To access 0x1F800, send 0x7E00
    To access 0x04000, send 0x1000
```

### STM32F3xx (64KB Flash, 2KB pages)

```
┌──────────────────────────────────────────────────────────────┐
│                    F3xx Memory Map (64KB)                    │
├──────────────────────────────────────────────────────────────┤
│                                                              │
│  0x0000 ┌─────────────────────────────────────────────────┐  │
│         │                                                 │  │
│         │              Bootloader                         │  │
│         │                 4KB                             │  │
│         │                                                 │  │
│  0x1000 ├─────────────────────────────────────────────────┤  │
│         │                                                 │  │
│         │              Firmware                           │  │
│         │                58KB                             │  │
│         │                                                 │  │
│  0xF800 ├─────────────────────────────────────────────────┤  │
│         │  Config (48B) │ Melody (128B) │ Reserved (80B)  │  │
│  0xF8FF └─────────────────────────────────────────────────┘  │
│                                                              │
└──────────────────────────────────────────────────────────────┘

Protocol addresses = Real addresses (no translation needed)
```

---

## EEPROM/Config Layout

Located at `eeprom_address` for each MCU:

```
Offset   Size   Content
───────────────────────────────────
+0       48     Configuration data
+48      128    Melody/music buffer
+176     80     Reserved
───────────────────────────────────
Total:   256    bytes
```

---

## Rust Implementation

```rust
// crates/esc-protocol/src/device.rs

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum McuType {
    /// STM32F051 - 32KB flash, 1KB pages, no address division
    F051,
    /// STM32G071 - 128KB flash, 2KB pages, addresses divided by 4
    G071,
    /// STM32F3xx - 64KB flash, 2KB pages, no address division
    F3xx,
    /// Unknown MCU type
    Unknown(u8),
}

impl McuType {
    /// Parse MCU type from device ID byte (from connect response)
    pub fn from_device_byte(byte: u8) -> Self {
        match byte {
            0x1F => McuType::F051,
            0x2B => McuType::G071,
            0x35 => McuType::F3xx,
            other => McuType::Unknown(other),
        }
    }

    /// Get the device ID byte
    pub fn device_id(&self) -> u8 {
        match self {
            McuType::F051 => 0x1F,
            McuType::G071 => 0x2B,
            McuType::F3xx => 0x35,
            McuType::Unknown(id) => *id,
        }
    }

    /// Total flash size in bytes
    pub fn flash_size(&self) -> usize {
        match self {
            McuType::F051 => 32 * 1024,
            McuType::G071 => 128 * 1024,
            McuType::F3xx => 64 * 1024,
            McuType::Unknown(_) => 32 * 1024,
        }
    }

    /// Flash page size in bytes
    pub fn page_size(&self) -> usize {
        match self {
            McuType::F051 => 1024,
            McuType::G071 | McuType::F3xx => 2048,
            McuType::Unknown(_) => 1024,
        }
    }

    /// EEPROM/config base address (protocol address)
    pub fn eeprom_address(&self) -> u16 {
        match self {
            McuType::F051 => 0x7C00,
            McuType::G071 => 0x7E00,  // Real: 0x1F800
            McuType::F3xx => 0xF800,
            McuType::Unknown(_) => 0x7C00,
        }
    }

    /// Whether addresses need to be divided by 4 for protocol
    pub fn needs_address_division(&self) -> bool {
        matches!(self, McuType::G071)
    }

    /// Translate real address to protocol address
    pub fn real_to_protocol(&self, real_address: u32) -> u16 {
        if self.needs_address_division() {
            (real_address >> 2) as u16
        } else {
            real_address as u16
        }
    }

    /// Translate protocol address to real address
    pub fn protocol_to_real(&self, protocol_address: u16) -> u32 {
        if self.needs_address_division() {
            (protocol_address as u32) << 2
        } else {
            protocol_address as u32
        }
    }

    /// Firmware start address (real address)
    pub fn firmware_start_real(&self) -> u32 {
        match self {
            McuType::G071 => 0x4000,  // 16KB bootloader
            _ => 0x1000,               // 4KB bootloader
        }
    }

    /// Firmware start address (protocol address)
    pub fn firmware_start_protocol(&self) -> u16 {
        self.real_to_protocol(self.firmware_start_real())
    }

    /// Maximum firmware size in bytes
    pub fn max_firmware_size(&self) -> usize {
        let eeprom_real = self.protocol_to_real(self.eeprom_address()) as usize;
        eeprom_real - self.firmware_start_real() as usize
    }
}

impl std::fmt::Display for McuType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McuType::F051 => write!(f, "STM32F051 (32KB, 1KB pages)"),
            McuType::G071 => write!(f, "STM32G071 (128KB, 2KB pages, addr/4)"),
            McuType::F3xx => write!(f, "STM32F3xx (64KB, 2KB pages)"),
            McuType::Unknown(id) => write!(f, "Unknown (0x{:02X})", id),
        }
    }
}
```

---

## Address Translation Examples

### G071 (addresses divided by 4)

| Real Address | Protocol Address | Description |
|--------------|------------------|-------------|
| 0x00000 | 0x0000 | Bootloader start |
| 0x04000 | 0x1000 | Firmware start |
| 0x1F800 | 0x7E00 | EEPROM/config |
| 0x1F830 | 0x7E0C | Melody buffer |

### F051/F3xx (no division)

| Real Address | Protocol Address | Description |
|--------------|------------------|-------------|
| 0x0000 | 0x0000 | Bootloader start |
| 0x1000 | 0x1000 | Firmware start |
| 0x7C00 (F051) | 0x7C00 | EEPROM/config |
| 0xF800 (F3xx) | 0xF800 | EEPROM/config |

---

## C++ Reference

From `widget.cpp`:

```cpp
void Widget::readInitData() {
    if (data[4] == (char)0x2b) {  // G071
        qInfo("G071ESC_2KB_PAGE");
        four_way->memory_divider_required_four = true;
        four_way->eeprom_address = 0x7e00;
    }
    if (data[4] == (char)0x1f) {  // F051
        qInfo("F0ESC_1KB_PAGE");
        four_way->memory_divider_required_four = false;
        four_way->eeprom_address = 0x7c00;
    }
    if (data[4] == (char)0x35) {  // F3
        qInfo("F3ESC_2KB_PAGE");
        four_way->memory_divider_required_four = false;
        four_way->eeprom_address = 0xF800;
    }
}

// Address translation during flash:
if (four_way->memory_divider_required_four) {
    writeData(four_way->makeFourWayWriteCommand(
        data, size, (4096 + (i * 2048) + (j * chunk_size)) >> 2));
} else {
    writeData(four_way->makeFourWayWriteCommand(
        data, size, 4096 + (i * 2048) + (j * chunk_size)));
}
```
