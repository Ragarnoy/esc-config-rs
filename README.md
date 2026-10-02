# esc-config-rs

Command-line tools and libraries for configuring AM32 electronic speed controllers.

## Supported Hardware

- AM32 (STM32/AT32 based ESCs)

BLHeli_S (SiLabs EFM8) and BLHeli_32 are not supported. BLHeli_S ESCs are
recognised at connect and refused.

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

# Leave the bootloader and start the ESC firmware
esc-configurator run -d /dev/ttyACM0

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
2. **Relay firmware** (`esc-relay-fw`) - Runs on RP2350, runs the ESC session and
   serves typed operations over USB
3. **Libraries** (`esc-protocol`, `esc-session`, `esc-config-icd`, `esc-onewire-rp`,
   `rtttl-rs`) - no_std; a flight controller embeds `esc-session` and the ICD types

```
PC (CLI) --USB CDC: postcard-rpc, COBS--> Relay (RP2350) --one-wire 19200--> ESC
```

All bootloader sequencing and timing lives in `esc-session`, which runs over any
half-duplex `Link`. The relay runs it over a PIO UART and answers one request per
operation (connect, read/write settings, flash a chunk, run), so USB latency never
touches the bootloader protocol. The relay enumerates as `1209:0001`; with one relay
plugged in, interactive mode finds it without `-d`.

## Crates

| Crate | Description | no_std |
|-------|-------------|--------|
| `esc-protocol` | AM32 bootloader frames, CRC-16, device/layout, EEPROM layout, melody, Intel HEX | Yes |
| `esc-session` | Bootloader session (connect, read, write, settings, flash) over a `Link` | Yes |
| `esc-config-icd` | Typed ESC operations as postcard-rpc endpoints | Yes |
| `esc-onewire-rp` | RP2040/RP235x PIO one-wire UART implementing `Link` | Yes |
| `esc-relay-fw` | Embassy firmware for RP2350 | N/A |
| `rtttl-rs` | RTTTL (Ring Tone Text Transfer Language) parser | Yes |
| `esc-configurator-cli` | CLI with interactive TUI | No |

## Hardware Setup

Connect the relay MCU to your ESC signal wire and power the relay first: the AM32
bootloader stays active when the signal line is driven high at power-on, which the
relay does. A signal held low for 20 ms makes it start the ESC firmware.

The relay is not a USB-serial bridge any more: tools that speak the bootloader
protocol themselves (am32.ca direct-connect) do not work through it.

Pin assignments for RP2350 (configurable in firmware source):
- GPIO15: ESC signal wire (one-wire half-duplex UART) — any GPIO can be used
- USB: CDC serial to PC

## Protocol Reference

See `docs/REVAMP.md`, "How the bootloader actually behaves", checked against the AM32
bootloader source. In short: 19200 8N1 one-wire, CRC-16/IBM (0xA001, LSB first);
frames end on idle line; writes are Set Address → Set Buffer (no reply) → payload +
CRC → Prog Flash; the 192-byte settings block is always written whole; EEPROM byte 0
must be `0x01` for the ESC firmware to start.

## License

MIT
