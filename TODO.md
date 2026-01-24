# ESC Config RS - TODO

Implementation roadmap based on comparison with reference C++ ESC Config Tool.

See `docs/` folder for detailed implementation guides.

---

## High Priority

| # | Feature | Doc | Status |
|---|---------|-----|--------|
| 1 | Four-Way Interface Protocol | [protocol-fourway.md](docs/protocol-fourway.md) | [x] |
| 2 | MSP Protocol | [protocol-msp.md](docs/protocol-msp.md) | [x] |
| 3 | Multiple ESC Memory Layouts | [memory-layout.md](docs/memory-layout.md) | [x] |
| 4 | Firmware Flashing | [flash-implementation.md](docs/flash-implementation.md) | [x] |
| 5 | Missing AM32 Config Fields | [config-fields.md](docs/config-fields.md) | [x] |

## Medium Priority

| # | Feature | Doc | Status |
|---|---------|-----|--------|
| 6 | Music/Melody Support | [features.md](docs/features.md#melody) | [x] |
| 7 | Default EEPROM Presets | [features.md](docs/features.md#presets) | [ ] |
| 8 | Safety Bit Handling | [flash-implementation.md](docs/flash-implementation.md#safety) | [x] |

## Nice to Have

| # | Feature | Doc | Status |
|---|---------|-----|--------|
| 9 | MSP Motor Control | [protocol-msp.md](docs/protocol-msp.md#motor-control) | [x] |
| 10 | Offline Config Editing | [features.md](docs/features.md#offline) | [x] |
| 11 | Firmware Verification | [flash-implementation.md](docs/flash-implementation.md#verification) | [x] |
| 12 | Config Serialization (serde/TOML) | - | [x] |
| 13 | ESC Diagnostics & Recovery | - | [ ] |

## DShot Features (Relay Firmware)

| # | Feature | Status | Notes |
|---|---------|--------|-------|
| D1 | DShot TX (300/600) | [x] | Beep commands working |
| D2 | Spin Direction Commands | [ ] | Commands 20/21 + SaveSettings (12) |
| D3 | 3D Mode Toggle | [ ] | Commands 9/10 |
| D4 | LED Control | [ ] | Commands 22-29 (if ESC has LEDs) |
| D5 | Extended Telemetry (EDT) | [ ] | Requires bidirectional DShot (RX PIO + GCR decode) |

### EDT Implementation Notes
- Requires RX PIO program to read inverted pulse widths
- GCR 5-to-4 bit decoding (21 bits → 16 data + CRC)
- Pin direction switching: TX → high-Z → RX → TX
- ~25µs turnaround, ~30µs response window
- Provides: eRPM, voltage, current, temperature

---

## Quick Reference

### Protocol Docs
- [protocol-msp.md](docs/protocol-msp.md) - Betaflight MSP protocol
- [protocol-fourway.md](docs/protocol-fourway.md) - Four-Way Interface (through FC)
- [protocol-direct.md](docs/protocol-direct.md) - Direct bootloader commands

### Implementation Docs
- [memory-layout.md](docs/memory-layout.md) - MCU types, addresses, memory maps
- [config-fields.md](docs/config-fields.md) - Complete EEPROM field reference
- [flash-implementation.md](docs/flash-implementation.md) - Flashing sequence
- [features.md](docs/features.md) - Other features (melody, presets, offline)

---

## Files to Create

```
crates/esc-protocol/src/
├── msp.rs              # MSP protocol ✅
├── fourway.rs          # Four-Way Interface ✅
├── fourway_session.rs  # Four-Way state machine ✅
├── melody.rs           # Melody format ✅
└── presets.rs          # Default configs

crates/rtttl-rs/src/
└── lib.rs              # RTTTL parser (separate crate) ✅

crates/esc-configurator-cli/src/
└── passthrough.rs  # FC passthrough mode
```

## Files to Update

```
crates/esc-protocol/src/
├── lib.rs          # Added module declarations ✅
├── device.rs       # Add McuType enum (FourWayMcuType added in fourway.rs) ✅
├── packet.rs       # Configurable addresses
└── am32_config.rs  # Missing fields ✅, serde (Am32ConfigData) ✅, BootStatus ✅

crates/esc-configurator-cli/src/
├── commands.rs     # Flash, verify, diagnose implementations (serde/toml added ✅)
└── relay.rs        # write_chunk, read_eeprom, etc.
```

---

## Feature Details

### ESC Diagnostics & Recovery (#13)

Protocol support is in place (`BootStatus` enum in `am32_config.rs`). CLI commands needed:

**`esc-configurator diagnose`** - Check ESC health:
- Boot status (Bootable / StuckInBootloader / Unknown)
- Firmware version
- EEPROM version
- Connection status

**`esc-configurator recover`** - Attempt to fix stuck ESC:
- Restore boot byte if stuck in bootloader
- Option to reset config to defaults

**Usage scenarios:**
1. ESC stuck after failed flash → `recover` restores boot byte
2. ESC behaving oddly → `diagnose` checks config integrity
3. Pre-flash health check → `diagnose` before firmware update
