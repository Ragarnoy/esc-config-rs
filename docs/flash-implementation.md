# Firmware Flashing Implementation

Complete guide for implementing firmware flashing functionality.

---

## Flash Sequence Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                      Firmware Flash Sequence                     │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  1. SAFETY: Clear boot byte                                     │
│     ├─ Read current EEPROM (48 bytes)                           │
│     ├─ Set byte[0] = 0x00                                       │
│     └─ Write back to EEPROM                                     │
│        (ESC will NOT boot if flash fails now)                   │
│                                                                  │
│  2. Load firmware file                                          │
│     ├─ .hex: Parse Intel HEX format                             │
│     └─ .bin: Read raw binary                                    │
│                                                                  │
│  3. Flash loop (chunk_size = 128 bytes)                         │
│     │                                                            │
│     │  for page in 0..=(firmware_size / 2048):                  │
│     │      for chunk in 0..(2048 / 128):                        │
│     │          │                                                 │
│     │          │  address = firmware_start + page*2048 + chunk*128│
│     │          │  if mcu.needs_address_division():              │
│     │          │      address = address >> 2                    │
│     │          │                                                 │
│     │          │  retry_count = 0                               │
│     │          │  loop:                                         │
│     │          │      send_chunk(address, data[128])            │
│     │          │      if ACK_OK: break                          │
│     │          │      retry_count += 1                          │
│     │          │      if retry_count > 8: return ERROR          │
│     │          │                                                 │
│     │          │  update_progress()                             │
│     │                                                            │
│  4. SAFETY: Restore boot byte                                   │
│     ├─ Set byte[0] = 0x01                                       │
│     └─ Write back to EEPROM                                     │
│        (ESC can now boot)                                       │
│                                                                  │
│  5. Optionally: Write music buffer (if present)                 │
│                                                                  │
│  6. Reset ESC                                                   │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

---

## Safety Bit Handling {#safety}

**CRITICAL:** The boot byte at EEPROM offset 0 must be managed correctly to prevent bricking.

### Before Flashing
```rust
// Read current EEPROM
let mut eeprom = relay.read_eeprom(48)?;

// Clear boot byte - ESC won't try to boot corrupted firmware
eeprom[0] = 0x00;
relay.write_eeprom(&eeprom)?;
```

### After Successful Flash
```rust
// Restore boot byte - ESC can now boot
eeprom[0] = 0x01;
relay.write_eeprom(&eeprom)?;
```

### On Flash Failure
```rust
// ALWAYS try to restore boot byte, even on error
// This gives the user another chance to flash
if let Err(e) = relay.write_chunk(...) {
    // Try to restore boot byte before propagating error
    eeprom[0] = 0x01;
    let _ = relay.write_eeprom(&eeprom);  // Ignore error here
    return Err(e);
}
```

---

## Write Chunk Sequence

Each 128-byte chunk requires 4 protocol operations:

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

**CRITICAL:** Step 2 has NO response. Do not wait for ACK.

---

## Address Calculation

```rust
fn calculate_chunk_address(
    mcu: &McuType,
    chunk_index: usize,
    chunk_size: usize,
) -> u16 {
    let firmware_start = mcu.firmware_start_real();
    let real_address = firmware_start + (chunk_index * chunk_size) as u32;

    // G071 requires address division by 4
    mcu.real_to_protocol(real_address)
}
```

---

## Rust Implementation

```rust
// crates/esc-configurator-cli/src/commands.rs

use indicatif::{ProgressBar, ProgressStyle};

pub fn flash_firmware(device: &str, hex_file: &str) -> Result<()> {
    // Load firmware file
    let firmware = load_firmware_file(hex_file)?;
    println!("Firmware size: {} bytes", firmware.len());

    // Validate size
    let mut relay = Relay::open(device)?;
    relay.connect()?;

    let mcu_type = relay.mcu_type();
    if firmware.len() > mcu_type.max_firmware_size() {
        bail!(
            "Firmware too large: {} bytes, max {} bytes for {}",
            firmware.len(),
            mcu_type.max_firmware_size(),
            mcu_type
        );
    }

    // Step 1: Clear boot byte (SAFETY)
    println!("Preparing ESC for flash...");
    let mut eeprom = relay.read_eeprom(48)?;
    let original_boot_byte = eeprom[0];
    eeprom[0] = 0x00;  // Clear boot byte
    relay.write_eeprom(&eeprom)?;

    // Step 2: Flash firmware in chunks
    let chunk_size = 128usize;
    let total_chunks = (firmware.len() + chunk_size - 1) / chunk_size;
    let max_retries = 8;

    let pb = ProgressBar::new(total_chunks as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
        .unwrap()
        .progress_chars("#>-"));

    println!("Flashing {} chunks...", total_chunks);

    for chunk_idx in 0..total_chunks {
        let start = chunk_idx * chunk_size;
        let end = (start + chunk_size).min(firmware.len());
        let chunk = &firmware[start..end];

        let protocol_addr = calculate_chunk_address(&mcu_type, chunk_idx, chunk_size);

        // Retry loop
        let mut retries = 0;
        loop {
            match relay.write_chunk(protocol_addr, chunk) {
                Ok(()) => break,
                Err(e) if retries < max_retries => {
                    retries += 1;
                    pb.println(format!("Retry {}/{} at 0x{:04X}: {}",
                        retries, max_retries, protocol_addr, e));
                }
                Err(e) => {
                    // CRITICAL: Try to restore boot byte
                    pb.finish_with_message("FAILED - restoring boot byte");
                    eeprom[0] = original_boot_byte;
                    let _ = relay.write_eeprom(&eeprom);
                    bail!("Flash failed at 0x{:04X}: {}", protocol_addr, e);
                }
            }
        }
        pb.inc(1);
    }
    pb.finish_with_message("Flash complete");

    // Step 3: Restore boot byte
    println!("Finalizing...");
    eeprom[0] = 0x01;
    relay.write_eeprom(&eeprom)?;

    // Step 4: Reset ESC
    relay.reset_esc()?;
    relay.disconnect()?;

    println!("Firmware flashed successfully!");
    Ok(())
}

fn load_firmware_file(path: &str) -> Result<Vec<u8>> {
    if path.ends_with(".hex") {
        let content = fs::read_to_string(path)?;
        let firmware = esc_protocol::ihex::load_firmware(&content)?;
        // Flatten segments into contiguous binary
        Ok(firmware.to_binary())
    } else {
        // Assume raw binary
        Ok(fs::read(path)?)
    }
}
```

---

## Relay Write Chunk Method

```rust
// crates/esc-configurator-cli/src/relay.rs

impl Relay {
    /// Write a chunk to flash memory
    pub fn write_chunk(&mut self, address: u16, data: &[u8]) -> Result<()> {
        // 1. Set address
        let addr_cmd = packet::set_address(address);
        self.send(&addr_cmd)?;
        let ack = self.recv_ack(Duration::from_millis(50))?;
        if ack != 0x30 {
            bail!("Set address failed: ACK={:02X}", ack);
        }

        // 2. Set buffer size (NO ACK!)
        let size_cmd = packet::set_buffer_size(data.len() as u16);
        self.send(&size_cmd)?;
        std::thread::sleep(Duration::from_millis(25));  // Just wait, no response

        // 3. Send data buffer
        let buffer_cmd = packet::send_buffer(data);
        self.send(&buffer_cmd)?;
        let ack = self.recv_ack(Duration::from_millis(75))?;
        if ack != 0x30 {
            bail!("Send buffer failed: ACK={:02X}", ack);
        }

        // 4. Commit flash
        let write_cmd = packet::write_flash();
        self.send(&write_cmd)?;
        let ack = self.recv_ack(Duration::from_millis(50))?;
        if ack != 0x30 {
            bail!("Write flash failed: ACK={:02X}", ack);
        }

        Ok(())
    }
}
```

---

## Firmware Verification {#verification}

After flashing, optionally verify the written data:

```rust
pub fn verify_firmware(device: &str, hex_file: &str) -> Result<()> {
    let firmware = load_firmware_file(hex_file)?;

    let mut relay = Relay::open(device)?;
    relay.connect()?;

    let mcu_type = relay.mcu_type();
    let chunk_size = 128;
    let total_chunks = (firmware.len() + chunk_size - 1) / chunk_size;

    let pb = ProgressBar::new(total_chunks as u64);
    println!("Verifying {} chunks...", total_chunks);

    for chunk_idx in 0..total_chunks {
        let start = chunk_idx * chunk_size;
        let end = (start + chunk_size).min(firmware.len());
        let expected = &firmware[start..end];

        let protocol_addr = calculate_chunk_address(&mcu_type, chunk_idx, chunk_size);
        let actual = relay.read_chunk(protocol_addr, expected.len())?;

        if actual != expected {
            pb.finish_with_message("VERIFICATION FAILED");
            bail!(
                "Mismatch at 0x{:04X}:\n  Expected: {:02X?}...\n  Actual:   {:02X?}...",
                protocol_addr,
                &expected[..8.min(expected.len())],
                &actual[..8.min(actual.len())]
            );
        }
        pb.inc(1);
    }

    pb.finish_with_message("Verification passed");
    relay.disconnect()?;
    println!("Firmware verified successfully!");
    Ok(())
}
```

---

## C++ Reference

From `widget.cpp`:

```cpp
void Widget::on_writeBinary_clicked() {
    uint16_t chunk_size = 128;

    // Clear safety bit
    eeprom_out[0] = 0x00;
    sendDirect(eeprom_out, 48, four_way->eeprom_address);

    // Flash loop
    for (int i = 0; i <= pages; i++) {
        for (int j = 0; j < 2048 / chunk_size; j++) {
            QByteArray chunk;
            // ... fill chunk ...

            retries = 0;
            four_way->ack_required = true;
            while (four_way->ack_required) {
                if (four_way->memory_divider_required_four) {
                    sendDirect(chunk, chunk.size(),
                        (4096 + (i * 2048) + (j * chunk_size)) >> 2);
                } else {
                    sendDirect(chunk, chunk.size(),
                        4096 + (i * 2048) + (j * chunk_size));
                }
                retries++;
                if (retries > max_retries) break;
            }

            if (four_way->ack_type == BAD_ACK) {
                ui->escStatusLabel_2->setText("FLASH FAILURE");
                return;
            }

            ui->progressBar->setValue((index * 100) / sizeofBin);
        }
    }

    // Restore safety bit
    eeprom_out[0] = 0x01;
    sendDirect(eeprom_out, 48, four_way->eeprom_address);
}
```
