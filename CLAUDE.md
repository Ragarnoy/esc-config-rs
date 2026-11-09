# CLAUDE.md

This file provides guidance for Claude Code when working on this project.

## Project Overview

ESC configurator toolchain for AM32 electronic speed controllers. Does NOT support BLHeli_S
(recognised at connect and refused) or BLHeli_32.

## Current work: revamp and audit

**Read [`docs/REVAMP.md`](docs/REVAMP.md) before changing anything.** The crates are
being reshaped so a flight controller (Elle, `../elle`, proposal
`docs/changes/0002-esc-config-over-rpc.md`) can embed the ESC session logic:

- The bootloader sequencing lives in `esc-session`: `no_std`, async, hardware-free,
  generic over a small `Link` trait, tested against a simulated AM32 bootloader
  (`crates/esc-session/tests/fake_esc`). Protocol rules belong there, not in callers.
- The relay firmware runs the session itself and serves typed operations
  (`esc-config-icd`, postcard-rpc over USB CDC with COBS frames, the same framing as
  Elle). No bootloader frame crosses USB.
- Work goes phase by phase (baseline → audit → core extraction → firmware/CLI → hand-off).
- Protocol facts come from the AM32 bootloader source and are written down in
  `docs/REVAMP.md` ("How the bootloader actually behaves"); the pre-audit protocol
  docs were deleted. `docs/config-fields.md` is the EEPROM field reference.
- Remaining: the bench regression (backup / set / restore / flash on a real ESC,
  compared with pre-revamp backups) and the Phase 1 bench checklist, then Phase 4
  (hand-off to Elle).

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
├── esc-protocol/      # AM32 bootloader protocol, pure logic (no_std)
│   ├── crc.rs         # CRC-16/IBM implementation
│   ├── packet.rs      # Frame builders and reply parsers
│   ├── device.rs      # Classification from the connect reply (AM32; SiLabs refused)
│   ├── layout.rs      # Flash layout in Set Address units (devinfo / flash code)
│   ├── am32_config.rs # AM32 EEPROM settings layout
│   ├── ihex.rs        # Intel HEX file parser
│   └── melody.rs      # Bluejay melody format + RTTTL conversion
├── esc-session/       # Bootloader session over a `Link` (no_std, async)
│   ├── link.rs        # `Link` trait, `Duration`
│   ├── session.rs     # `Esc`: connect, read, write, settings, flash, run
│   └── tests/         # Simulated AM32 bootloader + session tests
├── esc-config-icd/    # Typed ESC operations as postcard-rpc endpoints (no_std)
├── esc-onewire-rp/    # RP2040/RP235x PIO one-wire UART, an esc-session `Link`
├── esc-relay-fw/      # Embassy firmware for RP2350 (not a default member)
│   ├── main.rs        # USB CDC-ACM, PIO UART, postcard-rpc server
│   ├── app.rs         # ICD handlers: each runs one whole ESC operation
│   ├── wire.rs        # postcard-rpc WireTx/WireRx over CDC, COBS-framed
│   └── frame_buf.rs   # COBS frame reassembly (copied from Elle)
├── rtttl-rs/          # RTTTL parser (no_std, separate crate)
│   └── lib.rs         # Ring Tone Text Transfer Language parser
└── esc-configurator-cli/  # CLI tool (std)
    ├── main.rs        # Clap CLI definition
    ├── commands.rs    # Command implementations
    ├── device.rs      # `EscDevice` (one method per ICD op) + `Session` host policy
    ├── relay.rs       # `RelayClient`: EscDevice over postcard-rpc (cobs-serial)
    └── ui.rs          # Terminal UI (dialoguer)
```

## Key Constraints

- `esc-protocol`, `esc-session`, `esc-config-icd`, `esc-onewire-rp`, `rtttl-rs` must
  remain `#![no_std]`, no alloc; check with
  `cargo build -p <crate> --target thumbv8m.main-none-eabihf`
- No hardcoded clock rates in firmware code: derive PIO dividers from `clk_sys_freq()`
  (Elle runs at 200 MHz, the relay at 150 MHz)
- Use `heapless` collections instead of `std::vec::Vec` in no_std crates
- BLHeli bootloader uses 19200 baud, CRC-16/IBM polynomial 0xA001
- Edition is 2024

## Architecture

```
PC (CLI) --USB CDC: postcard-rpc, COBS--> Relay (RP2350) --one-wire 19200--> ESC
           esc-config-icd requests            esc-session + esc-onewire-rp
```

The relay runs each operation end to end; the CLI holds host policy (which bytes an
edit may touch, chunking a firmware image). Elle will run the same session and ICD
types over RTT. USB VID:PID `1209:0001`. The ESC signal pin is GPIO15.
The ESC enters its bootloader when the signal line is **driven high** at power-on
(and leaves it if the line is held low for 20 ms).

## Protocol Details

The authoritative summary is `docs/REVAMP.md`, "How the bootloader actually behaves".
The essentials:

- CRC-16/IBM (poly 0xA001, init 0), LSB first, on every command frame.
- Frames end on ~230 µs of idle line: bytes of a frame back to back, frames apart.
- Write: Set Address → Set Buffer (`FE 00 00 N`, or `FE 00 01 00` for 256; **no
  reply**) → ≥0.3 ms gap → payload + CRC (`30`, or `C2`: still waiting for a
  payload) → Prog Flash `01 01` (`30`). Lengths are multiples of 8. In payload mode
  any command frame is ACKed *as the payload* (zero-filled CRC check), so a Set
  Address there is silently swallowed; `Esc::write_at` relies on this to resync.
- A write at a page-aligned address erases the page, so the 192-byte settings
  structure (config 0–47, melody 48–175, CAN 176–183) is always written whole.
- EEPROM byte 0 must be `0x01` or the bootloader never starts the app.
- Addresses are shifted right by 2 on 128 KiB parts (G071); `esc_protocol::Layout`
  handles it. Run (`00 00 00 00`) leaves the bootloader.

## Testing

`cargo test`. Session behaviour is tested against the simulated bootloader in
`crates/esc-session/tests/fake_esc`; extend the fake when a new bootloader quirk is
found, citing the AM32 source it models.

## Conventions

- Keep clippy pedantic warnings minimal; fix relevant ones, ignore cosmetic ones
- Use `#[allow(dead_code)]` for unused protocol methods that are part of the complete API
- `async fn` in traits (`#[allow(async_fn_in_trait)]`) for `Link`-style traits
- Prefer embedded-hal traits over custom ones where possible

## Melody Format (Bluejay)

AM32 uses the Bluejay startup melody format, NOT a per-byte note encoding:
- Bytes 0-1: BPM (big-endian u16)
- Byte 2: default octave
- Byte 3: default duration
- Bytes 4+: pairs of (pulse_count, freq_index)
- freq_index 0 = silence (count = ms), nonzero = note (count = wave periods)
- Frequency formula: `1_000_000 / (24.72 * freq_index + 399.3)` Hz
- RTTTL format is supported for input, converted to Bluejay binary internally
