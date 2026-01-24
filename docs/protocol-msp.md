# MSP Protocol

MSP (MultiWii Serial Protocol) is used to communicate with Betaflight/INAV flight controllers.

## Packet Structure

### Request (Host to FC)
```
┌──────┬──────┬──────┬──────┬──────┬─────────────┬──────────┐
│ '$'  │ 'M'  │ '<'  │ LEN  │ CMD  │ PAYLOAD     │ CHECKSUM │
│ 0x24 │ 0x4D │ 0x3C │ u8   │ u8   │ [u8; LEN]   │ u8       │
└──────┴──────┴──────┴──────┴──────┴─────────────┴──────────┘
```

### Response (FC to Host)
```
┌──────┬──────┬──────┬──────┬──────┬─────────────┬──────────┐
│ '$'  │ 'M'  │ '>'  │ LEN  │ CMD  │ PAYLOAD     │ CHECKSUM │
│ 0x24 │ 0x4D │ 0x3E │ u8   │ u8   │ [u8; LEN]   │ u8       │
└──────┴──────┴──────┴──────┴──────┴─────────────┴──────────┘
```

### Checksum
XOR of all bytes from LEN onwards (LEN, CMD, and PAYLOAD).

```rust
fn msp_checksum(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |acc, &b| acc ^ b)
}
```

---

## Commands Used

| Command | Code | Payload | Description |
|---------|------|---------|-------------|
| MSP_API_VERSION | 0x68 | None | Query API version |
| MSP_SET_4WAY_IF | 0xF5 | None | Enter 4-way passthrough |
| MSP_SET_REBOOT | 0x44 | None | Reboot flight controller |
| MSP_SET_MOTOR | 0xD6 | 16 bytes | Set motor throttle |

---

## Rust Implementation

```rust
// crates/esc-protocol/src/msp.rs

pub const MSP_HEADER: [u8; 3] = [0x24, 0x4D, 0x3C]; // "$M<"
pub const MSP_RESPONSE_HEADER: [u8; 3] = [0x24, 0x4D, 0x3E]; // "$M>"

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum MspCommand {
    ApiVersion = 0x68,
    Set4WayIf = 0xF5,
    SetReboot = 0x44,
    SetMotor = 0xD6,
}

#[derive(Debug, Clone)]
pub struct MspResponse {
    pub cmd: u8,
    pub payload: Vec<u8>,
}

/// Build an MSP request packet
pub fn build_msp_packet(cmd: MspCommand, payload: &[u8]) -> Vec<u8> {
    let mut packet = Vec::with_capacity(6 + payload.len());
    packet.extend_from_slice(&MSP_HEADER);
    packet.push(payload.len() as u8);
    packet.push(cmd as u8);
    packet.extend_from_slice(payload);

    // XOR checksum from byte 3 onwards
    let checksum = packet[3..].iter().fold(0u8, |acc, &b| acc ^ b);
    packet.push(checksum);
    packet
}

/// Parse an MSP response packet
pub fn parse_msp_response(data: &[u8]) -> Option<MspResponse> {
    if data.len() < 6 {
        return None;
    }

    // Check header
    if &data[0..3] != &MSP_RESPONSE_HEADER {
        return None;
    }

    let len = data[3] as usize;
    let cmd = data[4];

    if data.len() < 6 + len {
        return None;
    }

    let payload = data[5..5 + len].to_vec();
    let checksum = data[5 + len];

    // Verify checksum
    let calculated = data[3..5 + len].iter().fold(0u8, |acc, &b| acc ^ b);
    if calculated != checksum {
        return None;
    }

    Some(MspResponse { cmd, payload })
}
```

---

## Motor Control {#motor-control}

Control motor throttle values through the flight controller.

### MSP_SET_MOTOR Packet

```
Command: 0xD6
Payload: 16 bytes
  ┌────────────┬────────────┬────────────┬────────────┬──────────┐
  │ Motor 1    │ Motor 2    │ Motor 3    │ Motor 4    │ Padding  │
  │ u16 LE     │ u16 LE     │ u16 LE     │ u16 LE     │ 8 bytes  │
  └────────────┴────────────┴────────────┴────────────┴──────────┘

Throttle values: 1000-2000 (PWM microseconds)
  - 1000 = motor off
  - 1500 = mid throttle
  - 2000 = full throttle
```

### Implementation

```rust
/// Build MSP motor control packet
pub fn build_motor_command(throttles: [u16; 4]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(16);
    for t in throttles {
        let clamped = t.clamp(1000, 2000);
        payload.push(clamped as u8);
        payload.push((clamped >> 8) as u8);
    }
    payload.extend_from_slice(&[0; 8]);  // Padding
    build_msp_packet(MspCommand::SetMotor, &payload)
}

// Usage example:
// Set motor 1 to 1100, others to 1000 (idle)
let cmd = build_motor_command([1100, 1000, 1000, 1000]);
```

---

## C++ Reference

From `widget.cpp`:

```cpp
void Widget::send_mspCommand(uint8_t cmd, QByteArray payload) {
    QByteArray mspMsgOut;
    mspMsgOut.append((char)0x24);  // '$'
    mspMsgOut.append((char)0x4d);  // 'M'
    mspMsgOut.append((char)0x3c);  // '<'
    mspMsgOut.append((char)payload.length());
    mspMsgOut.append((char)cmd);
    if (payload.length() > 0) {
        mspMsgOut.append(payload);
    }
    uint8_t checksum = 0;
    for (int i = 3; i < mspMsgOut.length(); i++) {
        checksum ^= mspMsgOut[i];
    }
    mspMsgOut.append((char)checksum);
    writeData(mspMsgOut);
}
```
