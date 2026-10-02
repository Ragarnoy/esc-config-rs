# Additional Features

Other features from the reference implementation.

---

## Melody/Music Support {#melody}

AM32 ESCs support custom startup melodies stored in EEPROM using the Bluejay format.

### Memory Location

```
Location: eeprom_address + 48
Size: 128 bytes
```

### Detection

If the first byte is `0xFF`, no melody is configured.

### Bluejay Binary Format

```
Offset  Size  Content
──────────────────────────────────
0       2     BPM (big-endian u16)
2       1     Default octave
3       1     Default duration (RTTTL: 1/2/4/8/16/32)
4+      2 ea  (pulse_count, freq_index) pairs
──────────────────────────────────
Max: 62 pairs (124 bytes of data after 4-byte header)
```

- `freq_index = 0`: silence, `pulse_count` = duration in milliseconds
- `freq_index != 0`: note, `pulse_count` = number of wave periods
- Frequency formula: `1_000_000 / (24.72 * freq_index + 399.3)` Hz
- Notes spanning > 255 pulses are split across multiple pairs
- Terminated by `pulse_count = 0`

### RTTTL Input Support

Melodies can be provided in RTTTL (Ring Tone Text Transfer Language) format,
which is converted to Bluejay binary automatically:

```
name:d=4,o=5,b=120:c,d,e,f,g
```

### EEPROM Write Strategy

AM32 flash writes erase the entire page. Writing melody alone at offset 48
would destroy config data. The CLI uses **read-modify-write**:

1. Read full 256-byte config block from EEPROM base
2. Overlay melody data at bytes 48-175
3. Write 176 bytes (config + melody) at EEPROM base address

This matches how am32.ca writes settings as a single block.

### CLI Commands

```bash
# Upload melody (RTTTL or binary)
esc-configurator melody upload -d /dev/ttyACM0 -i melody.rtttl

# Download melody from ESC
esc-configurator melody download -d /dev/ttyACM0 -o melody.bin

# Show current melody
esc-configurator melody show -d /dev/ttyACM0

# Clear melody
esc-configurator melody clear -d /dev/ttyACM0

# Show melody file info
esc-configurator melody info -f melody.rtttl

# Convert RTTTL to binary
esc-configurator melody convert -i melody.rtttl -o melody.bin
```

---

## Default EEPROM Presets {#presets}

Provide sensible default configurations for different use cases.

### Air/Multirotor Preset

Optimized for drones and aircraft:
- Fast response
- No braking (motors coast)
- Higher timing
- Standard startup power

### Crawler Preset

Optimized for RC cars and crawlers:
- Bidirectional enabled
- RC car reversing enabled
- Braking enabled
- Lower timing
- Lower startup power

### Rust Implementation

```rust
// crates/esc-protocol/src/presets.rs

/// Air/multirotor preset
pub const AIR_PRESET: [u8; 48] = [
    0x01,  // 0x00: boot_byte
    0x03,  // 0x01: eeprom_version
    0x00,  // 0x02: bootloader_version (ESC fills)
    0x00,  // 0x03: firmware_major (ESC fills)
    0x00,  // 0x04: firmware_minor (ESC fills)
    0x64,  // 0x05: max_rate_of_change = 10.0
    0x00,  // 0x06: min_duty = 0%
    0x00,  // 0x07: disable_stick_calib = false
    0x00,  // 0x08: absolute_voltage = 0
    0x32,  // 0x09: current_limit_p = 100
    0x00,  // 0x0A: current_limit_i = 0
    0x32,  // 0x0B: current_limit_d = 100
    0x01,  // 0x0C: active_brake_strength = 1
    0x00, 0x00, 0x00, 0x00,  // reserved
    0x00,  // 0x11: direction_reversed = false
    0x00,  // 0x12: bidirectional = false
    0x00,  // 0x13: sinusoidal_startup = false
    0x00,  // 0x14: complementary_pwm = false
    0x00,  // 0x15: variable_pwm = fixed
    0x01,  // 0x16: stuck_rotor_protection = true
    0x1A,  // 0x17: timing_advance = 15°
    0x30,  // 0x18: pwm_frequency = 48kHz
    0x64,  // 0x19: startup_power = 100
    0x16,  // 0x1A: motor_kv = 880
    0x0E,  // 0x1B: motor_poles = 14
    0x00,  // 0x1C: brake_on_stop = off
    0x01,  // 0x1D: stall_protection = true
    0x05,  // 0x1E: beep_volume = 5
    0x00,  // 0x1F: telemetry_30ms = false
    0x80,  // 0x20: servo_low
    0x80,  // 0x21: servo_high
    0x80,  // 0x22: servo_neutral
    0x32,  // 0x23: servo_deadband = 50
    0x00,  // 0x24: low_voltage_cutoff = false
    0x00,  // 0x25: lvc_threshold
    0x00,  // 0x26: rc_car_reversing = false
    0x00,  // 0x27: use_hall_sensors = false
    0x0A,  // 0x28: sine_mode_range = 10
    0x05,  // 0x29: drag_brake_strength = 5
    0x05,  // 0x2A: running_brake_strength = 5
    0x8E,  // 0x2B: temperature_limit = disabled
    0x65,  // 0x2C: current_limit = disabled
    0x05,  // 0x2D: sine_mode_power = 5
    0x00,  // 0x2E: signal_type = auto
    0x00,  // 0x2F: auto_timing = false
];

/// Crawler preset
pub const CRAWLER_PRESET: [u8; 48] = [
    0x01,  // boot_byte
    0x03,  // eeprom_version
    0x00, 0x00, 0x00,  // versions (ESC fills)
    0x32,  // max_rate_of_change = 5.0 (slower)
    0x00,  // min_duty
    0x00,  // disable_stick_calib
    0x00,  // absolute_voltage
    0x32, 0x00, 0x32,  // current limit PID
    0x05,  // active_brake_strength = 5
    0x00, 0x00, 0x00, 0x00,  // reserved
    0x00,  // direction_reversed
    0x01,  // bidirectional = TRUE
    0x01,  // sinusoidal_startup = TRUE
    0x00,  // complementary_pwm
    0x00,  // variable_pwm
    0x01,  // stuck_rotor_protection
    0x12,  // timing_advance = lower
    0x18,  // pwm_frequency = 24kHz
    0x50,  // startup_power = 80 (lower)
    0x10,  // motor_kv
    0x0E,  // motor_poles = 14
    0x01,  // brake_on_stop = brake
    0x01,  // stall_protection
    0x05,  // beep_volume
    0x00,  // telemetry_30ms
    0x80, 0x80, 0x80, 0x32,  // servo settings
    0x00,  // low_voltage_cutoff
    0x00,  // lvc_threshold
    0x01,  // rc_car_reversing = TRUE
    0x00,  // use_hall_sensors
    0x0F,  // sine_mode_range = 15
    0x05,  // drag_brake_strength
    0x05,  // running_brake_strength
    0x8E,  // temperature_limit = disabled
    0x65,  // current_limit = disabled
    0x05,  // sine_mode_power
    0x00,  // signal_type = auto
    0x00,  // auto_timing
];

#[derive(Debug, Clone, Copy)]
pub enum Preset {
    Air,
    Crawler,
}

impl Preset {
    pub fn bytes(&self) -> &'static [u8; 48] {
        match self {
            Preset::Air => &AIR_PRESET,
            Preset::Crawler => &CRAWLER_PRESET,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Preset::Air => "Air/Multirotor",
            Preset::Crawler => "Crawler/RC Car",
        }
    }
}
```

### CLI Command

```bash
# Apply air preset
esc-configurator preset --device /dev/ttyACM0 --type air

# Apply crawler preset
esc-configurator preset --device /dev/ttyACM0 --type crawler
```

---

## Offline Config Editing {#offline}

Edit configuration files without a connected ESC.

### Use Cases

1. Prepare configuration before having the ESC
2. Create configuration templates
3. Batch configuration for multiple ESCs
4. Backup/restore without connection

### Rust Implementation

```rust
// crates/esc-configurator-cli/src/commands.rs

pub fn edit_offline(
    input: Option<&str>,
    preset: Option<Preset>,
    output: &str,
) -> Result<()> {
    // Load or create config
    let mut config = match (input, preset) {
        (Some(path), _) => {
            let data = fs::read(path)?;
            if data.len() != 48 && data.len() != 256 {
                bail!("Invalid config file size: {} bytes", data.len());
            }
            let mut arr = [0u8; 256];
            arr[..data.len()].copy_from_slice(&data);
            Am32Config::from_bytes(arr)
        }
        (None, Some(p)) => {
            let mut arr = [0u8; 256];
            arr[..48].copy_from_slice(p.bytes());
            Am32Config::from_bytes(arr)
        }
        (None, None) => {
            let mut arr = [0u8; 256];
            arr[..48].copy_from_slice(Preset::Air.bytes());
            Am32Config::from_bytes(arr)
        }
    };

    // Show current values
    println!("Current configuration:");
    println!("{}", config);

    // Interactive edit loop (reuse existing edit functions)
    let modified = edit_settings_loop(&mut config)?;

    if modified {
        // Save to file
        fs::write(output, &config.as_bytes()[..48])?;
        println!("Configuration saved to: {}", output);
    }

    Ok(())
}
```

### CLI Commands

```bash
# Create new config from air preset
esc-configurator edit-offline --preset air --output my_config.bin

# Edit existing config file
esc-configurator edit-offline --input my_config.bin --output my_config.bin

# Later, apply to ESC
esc-configurator restore --device /dev/ttyACM0 --input my_config.bin
```

---

## Reference: am32.ca Configurator

The am32.ca web configurator (am32-firmware/am32-configurator on GitHub)
writes all settings as a single block:

- **Layout size:** 184 bytes (0xB8) = config + melody + CAN settings
- **Melody offset:** 0x30 (48 bytes into the layout)
- **Melody size:** 128 bytes
- **Write approach:** Single write of full layout at EEPROM base address
- **No erase before write:** The bootloader's commit handles page erase internally

The C++ desktop configurator (ESC Config Tool) writes melody separately
at `eeprom_address + 48`, which can cause data loss due to page erase.
Our implementation follows the am32.ca approach.
