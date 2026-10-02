# CLAUDE.md

This file provides guidance for Claude Code when working on this project.

## Project Overview

ESC configurator toolchain for BLHeli_S and AM32 electronic speed controllers. Does NOT support BLHeli_32.

## Current work: revamp and audit

**Read [`docs/REVAMP.md`](docs/REVAMP.md) before changing anything.** The crates are
being reshaped so a flight controller (Elle, `../elle`, proposal
`docs/changes/0002-esc-config-over-rpc.md`) can embed the ESC session logic:

- The bootloader sequencing moves out of `esc-configurator-cli/src/relay.rs` into a
  `no_std`, async, hardware-free `esc-session` crate, generic over a small `Link` trait
  and tested against a simulated AM32 bootloader.
- The relay firmware stops pattern-matching the USB byte stream; it runs the session
  itself and speaks typed operations (shared ICD with Elle).
- Work goes phase by phase (baseline → audit → core extraction → firmware/CLI → hand-off).
  Phase 1 is read-only: findings go in `docs/REVAMP.md`, not straight into code.
- Several protocol statements in this file and in `docs/` contradict each other or the
  code (bootloader entry high vs low, Set Buffer ACK, timeouts, ESC pin). They are
  listed in `docs/REVAMP.md`; treat them as unverified until the audit settles them.
- Keep the CLI + relay working at the end of every phase; bench regression is
  backup / set / restore / flash on a real ESC, compared with pre-revamp backups.

## Build Commands

```bash
# Build all default crates (excludes embedded firmware)
cargo build

# Run tests
cargo test

# Clippy with pedantic lints
cargo clippy -- -W clippy::pedantic

# Build embedded firmware (requires ARM toolchain)
cargo build -p esc-relay-fw --target thumbv8m.main-none-eabihf
```

## Project Structure

```
crates/
├── esc-protocol/      # Core BLHeli protocol (no_std)
│   ├── crc.rs         # CRC-16/IBM implementation
│   ├── packet.rs      # Protocol packet builders
│   ├── bootloader.rs  # Bootloader state machine
│   ├── device.rs      # Device classification (SiLabs, ARM)
│   ├── config.rs      # Config block parsing
│   ├── am32_config.rs # AM32 configuration settings
│   ├── ihex.rs        # Intel HEX file parser
│   ├── melody.rs      # Bluejay melody format + RTTTL conversion
│   ├── msp.rs         # MSP protocol (Betaflight)
│   ├── fourway.rs     # Four-Way Interface protocol
│   └── fourway_session.rs # Four-Way state machine
├── esc-relay-core/    # Generic relay MCU logic (no_std)
│   ├── protocol.rs    # USB<->relay command protocol
│   ├── state.rs       # Relay state machine
│   └── traits.rs      # HAL trait re-exports
├── esc-relay-fw/      # Embassy firmware for RP2350
│   ├── main.rs        # Entry point, USB CDC-ACM, write handler
│   └── onewire_uart.rs # PIO-based half-duplex UART
├── rtttl-rs/          # RTTTL parser (no_std, separate crate)
│   └── lib.rs         # Ring Tone Text Transfer Language parser
└── esc-configurator-cli/  # CLI tool (std)
    ├── main.rs        # Clap CLI definition
    ├── commands.rs    # Command implementations
    ├── relay.rs       # Relay communication
    └── ui.rs          # Terminal UI (dialoguer)
```

## Key Constraints

- `esc-protocol`, `esc-relay-core`, `rtttl-rs` (and the future `esc-session`,
  `esc-config-icd`) must remain `#![no_std]`; check with
  `cargo build -p <crate> --target thumbv8m.main-none-eabihf`
- No hardcoded clock rates in firmware code: derive PIO dividers from `clk_sys_freq()`
  (Elle runs at 200 MHz, the relay at 150 MHz)
- Use `heapless` collections instead of `std::vec::Vec` in no_std crates
- BLHeli bootloader uses 19200 baud, CRC-16/IBM polynomial 0xA001
- Edition is 2024

## Architecture

```
PC (USB Serial) <-> Relay MCU (RP2350) <-> ESC
                         |
                         +-- UART 19200 (BLHeli/AM32 bootloader protocol)
```

The relay MCU acts as a USB-to-UART bridge for ESC bootloader communication. It handles write operations atomically to avoid USB latency causing CRC errors. The ESC signal pin is configurable (currently GPIO15, any GPIO works). The ESC must already be in bootloader mode (e.g. via DShot command from a flight controller, or power-on with signal held low).

## Protocol Details

### AM32 Bootloader Protocol

All commands use CRC-16/IBM (polynomial 0xA001, init 0x0000), sent LSB first.

**Read sequence (handled by passthrough):**
1. Set Address: `[FF 00 ADDR_H ADDR_L CRC_L CRC_H]` → ACK (0x30)
2. Read: `[03 LEN CRC_L CRC_H]` → `[DATA...] [CRC_L CRC_H] [ACK]`

**Write sequence (handled atomically by firmware):**
1. Set Address: `[FF 00 ADDR_H ADDR_L CRC_L CRC_H]` → ACK (0x30)
2. Set Buffer: `[FE 00 x256 LEN CRC_L CRC_H]` → **NO ACK**
3. Payload: `[DATA...] [CRC_L CRC_H]` → ACK (0x30)
4. Flash/Commit: `[01 01 CRC_L CRC_H]` → ACK (0x30)

**Critical:** The Set Buffer command does NOT receive an ACK. The firmware detects write transactions by looking for the `[FF 00 ... FE 00 00 ...]` combined pattern and handles steps 1-3 atomically.

### Relay Command Protocol
- Frame format: `[CMD:1] [LEN:1] [DATA:LEN] [CRC:1]`
- CRC is XOR of all preceding bytes
- Commands: Ping, Info, EnterBootloader, SendDshot, EnterPassthrough, ExitPassthrough, Reset

## Testing

All protocol logic has unit tests. Run with `cargo test`. Key test vectors for CRC-16/IBM are in `esc-protocol/src/crc.rs`.

## Conventions

- Keep clippy pedantic warnings minimal; fix relevant ones, ignore cosmetic ones
- Use `#[allow(dead_code)]` for unused protocol methods that are part of the complete API
- Use `async_fn_in_trait` (allowed in relay-core) for async HAL traits
- Prefer embedded-hal traits over custom ones where possible

## Key Implementation Notes

- The AM32 write sequence discovery: Set Buffer gets **NO ACK** - this was a critical finding
- The relay firmware handles writes atomically to avoid USB latency causing CRC errors
- Config address: 0x7C00 (48 bytes used, padded to 256)
- Arduino pattern for reads: 48 bytes at 0x7C00, then 128 bytes at 0x7C30
- The ESC signal pin (one-wire UART) is configurable; currently set to GPIO15 in the firmware

### EEPROM Write: Full-Block Read-Modify-Write

AM32 flash writes erase the entire flash page. Writing only config (48 bytes) or
only melody (128 bytes) at a sub-page offset destroys the rest of the page.

**Both `write_config` and `write_melody` must use read-modify-write:**
1. Read the full 256-byte config block from EEPROM
2. Overlay the changed portion (config bytes 0-47, or melody bytes 48-175)
3. Write 176 bytes (config + melody) at the EEPROM base address

This matches how am32.ca writes settings (184 bytes as a single block at eeprom base).
The ESC rejects writes of 256 bytes (NACK 0xC1), so we write 176 bytes (48+128).

### Melody Format (Bluejay)

AM32 uses the Bluejay startup melody format, NOT a per-byte note encoding:
- Bytes 0-1: BPM (big-endian u16)
- Byte 2: default octave
- Byte 3: default duration
- Bytes 4+: pairs of (pulse_count, freq_index)
- freq_index 0 = silence (count = ms), nonzero = note (count = wave periods)
- Frequency formula: `1_000_000 / (24.72 * freq_index + 399.3)` Hz
- RTTTL format is supported for input, converted to Bluejay binary internally
