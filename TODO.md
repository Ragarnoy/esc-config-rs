# ESC Config RS - TODO

## Completed

| # | Feature | Doc | Status |
|---|---------|-----|--------|
| 1 | Four-Way Interface Protocol | [protocol-fourway.md](docs/protocol-fourway.md) | [x] |
| 2 | MSP Protocol | [protocol-msp.md](docs/protocol-msp.md) | [x] |
| 3 | Multiple ESC Memory Layouts | [memory-layout.md](docs/memory-layout.md) | [x] |
| 4 | Firmware Flashing | [flash-implementation.md](docs/flash-implementation.md) | [x] |
| 5 | AM32 Config Fields | [config-fields.md](docs/config-fields.md) | [x] |
| 6 | Music/Melody Support (Bluejay + RTTTL) | [features.md](docs/features.md#melody) | [x] |
| 7 | Safety Bit Handling | [flash-implementation.md](docs/flash-implementation.md#safety) | [x] |
| 8 | MSP Motor Control | [protocol-msp.md](docs/protocol-msp.md#motor-control) | [x] |
| 9 | Offline Config Editing | [features.md](docs/features.md#offline) | [x] |
| 10 | Firmware Verification | [flash-implementation.md](docs/flash-implementation.md#verification) | [x] |
| 11 | Config Serialization (serde/TOML) | - | [x] |

## Remaining

| # | Feature | Notes |
|---|---------|-------|
| 1 | Default EEPROM Presets | [features.md](docs/features.md#presets) |
| 2 | ESC Diagnostics & Recovery | `BootStatus` enum exists; CLI commands needed |
| 3 | FC Passthrough Mode | 4-way IF through flight controller (needs `passthrough.rs`) |

### ESC Diagnostics & Recovery

Protocol support is in place (`BootStatus` enum in `am32_config.rs`). CLI commands needed:

- **`esc-configurator diagnose`** - Boot status, firmware version, EEPROM version
- **`esc-configurator recover`** - Restore boot byte if stuck in bootloader

---

## Reference Docs

- [protocol-msp.md](docs/protocol-msp.md) - Betaflight MSP protocol
- [protocol-fourway.md](docs/protocol-fourway.md) - Four-Way Interface (through FC)
- [protocol-direct.md](docs/protocol-direct.md) - Direct bootloader commands
- [memory-layout.md](docs/memory-layout.md) - MCU types, addresses, memory maps
- [config-fields.md](docs/config-fields.md) - Complete EEPROM field reference
- [flash-implementation.md](docs/flash-implementation.md) - Flashing sequence
- [features.md](docs/features.md) - Melody, presets, offline editing
