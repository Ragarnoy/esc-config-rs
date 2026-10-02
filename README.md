# esc-config-rs

Command-line tools and libraries for configuring BLHeli_S and AM32 electronic speed controllers.

## Supported Hardware

- BLHeli_S (SiLabs EFM8 based ESCs)
- AM32 (STM32/AT32 based ESCs)

BLHeli_32 is not supported.

## Requirements

- Rust 1.85+ (edition 2024)
- A relay probe MCU (RP2350/RP2040) flashed with the relay firmware
- For firmware development: ARM toolchain (`thumbv8m.main-none-eabihf` target)

## Building

```bash
# Build CLI and libraries
cargo build --release

# Run tests
cargo test

# Build relay firmware (requires ARM target installed)
rustup target add thumbv8m.main-none-eabihf
cargo build -p esc-relay-fw --target thumbv8m.main-none-eabihf --release
```

## Usage

```bash
# Interactive configuration mode (default)
esc-configurator

# List available serial ports
esc-configurator ports

# Show current ESC configuration (human-readable)
esc-configurator show -d /dev/ttyACM0

# Read ESC configuration (raw hex dump)
esc-configurator read -d /dev/ttyACM0

# Backup configuration to file
esc-configurator backup -d /dev/ttyACM0 -o backup.bin

# Restore configuration from backup
esc-configurator restore -d /dev/ttyACM0 -i backup.bin

# Set a specific setting
esc-configurator set -d /dev/ttyACM0 -s motor-kv -v 2200

# List available settings
esc-configurator settings

# Flash firmware
esc-configurator flash -d /dev/ttyACM0 -f firmware.hex

# Verify firmware matches hex file
esc-configurator verify -d /dev/ttyACM0 -f firmware.hex

# Display Intel HEX file info
esc-configurator hex-info -f firmware.hex

# Upload melody from RTTTL or binary file
esc-configurator melody upload -d /dev/ttyACM0 -i melody.rtttl

# Download melody from ESC
esc-configurator melody download -d /dev/ttyACM0 -o melody.bin

# Show current melody on ESC
esc-configurator melody show -d /dev/ttyACM0

# Clear melody
esc-configurator melody clear -d /dev/ttyACM0

# Show melody file info
esc-configurator melody info -f melody.rtttl

# Convert RTTTL to binary
esc-configurator melody convert -i melody.rtttl -o melody.bin
```

## Architecture

The system consists of three components:

1. **CLI tool** (`esc-configurator-cli`) - Runs on PC, communicates with relay over USB serial
2. **Relay firmware** (`esc-relay-fw`) - Runs on RP2350, bridges USB to ESC
3. **Protocol libraries** (`esc-protocol`, `rtttl-rs`) - Shared logic, no_std compatible

```
PC (USB) <---> Relay MCU <---> ESC
                  |
                  +-- UART 19200 (BLHeli/AM32 bootloader protocol)
```

The relay MCU acts as a USB-to-UART bridge for ESC bootloader communication. The ESC must already be in bootloader mode. For write operations, the relay handles the timing-critical sequence atomically to avoid USB latency issues.

## Crates

| Crate | Description | no_std |
|-------|-------------|--------|
| `esc-protocol` | BLHeli bootloader protocol, CRC-16, packet builders, AM32 config, melody | Yes |
| `esc-relay-core` | Generic relay state machine, HAL traits | Yes |
| `esc-relay-fw` | Embassy firmware for RP2350 | N/A |
| `rtttl-rs` | RTTTL (Ring Tone Text Transfer Language) parser | Yes |
| `esc-configurator-cli` | CLI with interactive TUI | No |

## Hardware Setup

Connect the relay MCU to your ESC signal wire. The ESC must be in bootloader mode
(e.g. via DShot command from a flight controller, or power-on with signal held low).

The relay handles:
- Half-duplex one-wire UART via PIO at 19200 baud
- Atomic write sequences for reliable config/melody updates
- USB CDC serial interface to PC

Pin assignments for RP2350 (configurable in firmware source):
- GPIO15: ESC signal wire (one-wire half-duplex UART) — any GPIO can be used
- USB: CDC serial to PC

## Protocol Reference

### AM32 Bootloader Protocol

- Baud rate: 19200 (one-wire half-duplex)
- CRC: CRC-16/IBM (polynomial 0xA001, init 0x0000, LSB first)

**Read sequence:**
1. Set Address: `[FF 00 ADDR_H ADDR_L CRC_L CRC_H]` → ACK (0x30)
2. Read: `[03 LEN CRC_L CRC_H]` → `[DATA...] [CRC_L CRC_H] [ACK]`

**Write sequence (critical timing):**
1. Set Address: `[FF 00 ADDR_H ADDR_L CRC_L CRC_H]` → ACK (0x30)
2. Set Buffer: `[FE 00 00 LEN CRC_L CRC_H]` → **NO ACK**
3. Payload: `[DATA...] [CRC_L CRC_H]` → ACK (0x30)
4. Flash/Commit: `[01 01 CRC_L CRC_H]` → ACK (0x30)

**Important:** The Set Buffer command (step 2) does NOT receive an ACK. The firmware handles steps 1-3 atomically without waiting for a response after Set Buffer.

## License

MIT
