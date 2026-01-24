# Direct Bootloader Protocol

Direct communication with AM32/BLHeli bootloader without going through a flight controller.
This is what the relay firmware uses.

---

## Commands

| Opcode | Name | Request Format | Response |
|--------|------|----------------|----------|
| 0xFF | Set Address | `[FF 00 ADDR_HI ADDR_LO CRC16]` | ACK (0x30) |
| 0xFE | Set Buffer Size | `[FE 00 00 SIZE CRC16]` | **NONE!** |
| 0x03 | Read Flash | `[03 SIZE CRC16]` | `[DATA... CRC16 ACK]` |
| 0x01 | Write/Commit | `[01 01 CRC16]` | ACK (0x30) |
| 0x02 | Erase Page | `[02 00 CRC16]` | ACK (0x30) |
| 0xFD | Keepalive | `[FD 00 CRC16]` | ACK (0xC1) |
| 0x00 | Disconnect | `[00 01 CRC16]` | ACK (0x30) |

**Note:** Buffer size = 0 means 256 bytes.

---

## ACK Values

| Value | Meaning |
|-------|---------|
| 0x30 | Success |
| 0xC1 | Keepalive ACK |
| Other | Error |

---

## CRC-16/IBM

- Polynomial: 0xA001 (bit-reversed 0x8005)
- Initial value: 0x0000
- Wire order: LSB first (little-endian)

```rust
pub fn crc16_ibm(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &byte in data {
        let mut x = byte;
        for _ in 0..8 {
            if ((x & 0x01) ^ (crc as u8 & 0x01)) != 0 {
                crc = (crc >> 1) ^ 0xA001;
            } else {
                crc >>= 1;
            }
            x >>= 1;
        }
    }
    crc
}
```

---

## Write Sequence

To write data to flash, you must follow this exact sequence:

```
┌─────────────────────────────────────────────────────────────────┐
│              Write Chunk Sequence                                │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  1. Set Address                                                 │
│     TX: [0xFF] [0x00] [ADDR_HI] [ADDR_LO] [CRC_LO] [CRC_HI]     │
│     RX: [0x30] (ACK)                                            │
│     Timeout: 50ms                                               │
│                                                                  │
│  2. Set Buffer Size                                             │
│     TX: [0xFE] [0x00] [0x00] [SIZE] [CRC_LO] [CRC_HI]           │
│         (SIZE=0 means 256)                                      │
│     RX: NONE! (AM32 quirk - no ACK for this command)            │
│     Wait: 25ms                                                  │
│                                                                  │
│  3. Send Data Buffer                                            │
│     TX: [DATA; SIZE] [CRC_LO] [CRC_HI]                          │
│     RX: [0x30] (ACK)                                            │
│     Timeout: 75ms                                               │
│                                                                  │
│  4. Commit Flash                                                │
│     TX: [0x01] [0x01] [CRC_LO] [CRC_HI]                         │
│     RX: [0x30] (ACK)                                            │
│     Timeout: 50ms                                               │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

**CRITICAL:** Step 2 (Set Buffer Size) has NO ACK response. This is an AM32 quirk.
Your code must NOT wait for a response after sending the buffer size command.

---

## Read Sequence

```
1. Set Address
   TX: [0xFF] [0x00] [ADDR_HI] [ADDR_LO] [CRC16]
   RX: [0x30]

2. Read Flash
   TX: [0x03] [SIZE] [CRC16]
   RX: [0x03] [SIZE] [DATA...] [CRC16] [0x30]
```

---

## Connect Sequence (Init)

The bootloader expects a specific init sequence:

```
TX: 12 zero bytes + [0x0D] "BLHeli" [0xF4] [0x7D]
    = [00 00 00 00 00 00 00 00 00 00 00 00 0D 42 4C 48 65 6C 69 F4 7D]

RX: Device info response (varies by MCU)
```

---

## Rust Implementation

From `BF_ROOTLOADER.cpp`:

```rust
// crates/esc-protocol/src/packet.rs

/// Set address command
pub fn set_address(address: u16) -> [u8; 6] {
    let mut packet = [0u8; 6];
    packet[0] = 0xFF;
    packet[1] = 0x00;
    packet[2] = (address >> 8) as u8;
    packet[3] = address as u8;
    let crc = crc16_ibm(&packet[..4]);
    packet[4] = crc as u8;
    packet[5] = (crc >> 8) as u8;
    packet
}

/// Set buffer size command (NO ACK RESPONSE!)
pub fn set_buffer_size(size: u16) -> [u8; 6] {
    let size_byte = if size == 256 { 0 } else { size as u8 };
    let mut packet = [0u8; 6];
    packet[0] = 0xFE;
    packet[1] = 0x00;
    packet[2] = 0x00;
    packet[3] = size_byte;
    let crc = crc16_ibm(&packet[..4]);
    packet[4] = crc as u8;
    packet[5] = (crc >> 8) as u8;
    packet
}

/// Read flash command
pub fn read_flash(size: u8) -> [u8; 4] {
    let mut packet = [0u8; 4];
    packet[0] = 0x03;
    packet[1] = size;
    let crc = crc16_ibm(&packet[..2]);
    packet[2] = crc as u8;
    packet[3] = (crc >> 8) as u8;
    packet
}

/// Write/commit flash command
pub fn write_flash() -> [u8; 4] {
    let mut packet = [0u8; 4];
    packet[0] = 0x01;
    packet[1] = 0x01;
    let crc = crc16_ibm(&packet[..2]);
    packet[2] = crc as u8;
    packet[3] = (crc >> 8) as u8;
    packet
}

/// Send data buffer with CRC
pub fn send_buffer(data: &[u8]) -> Vec<u8> {
    let mut packet = data.to_vec();
    let crc = crc16_ibm(&packet);
    packet.push(crc as u8);
    packet.push((crc >> 8) as u8);
    packet
}
```

---

## C++ Reference

From `BF_ROOTLOADER.cpp`:

```cpp
QByteArray BF_ROOTLOADER::setAddress(uint16_t address) {
    QByteArray thisAddress;
    thisAddress.append((char)0xff);
    thisAddress.append((char)0x00);
    thisAddress.append((char)((address >> 8) & 0xff));
    thisAddress.append((char)(address & 0xff));
    makeCRC(thisAddress, 4);
    thisAddress.append(calculated_crc_low_byte);
    thisAddress.append(calculated_crc_high_byte);
    return thisAddress;
}

QByteArray BF_ROOTLOADER::setBufferSize(uint16_t size) {
    if (size == 256) size = 0;  // 0 means 256!
    QByteArray output;
    output.append((char)0xfe);
    output.append((char)0x00);
    output.append((char)0x00);
    output.append((char)(uint8_t)size);
    makeCRC(output, 4);
    output.append((char)calculated_crc_low_byte);
    output.append((char)calculated_crc_high_byte);
    return output;
}
```

---

## Timeouts

| Operation | Timeout |
|-----------|---------|
| Connect | 2000ms |
| Set Address | 50ms |
| Set Buffer Size | 25ms (no response expected) |
| Send Buffer | 75ms |
| Write/Commit | 50ms |
| Read | 500ms |
| Default | 100ms |
