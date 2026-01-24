# Additional Features

Other features from the reference implementation.

---

## Melody/Music Support {#melody}

AM32 ESCs support custom startup melodies stored in EEPROM.

### Memory Location

```
Location: eeprom_address + 48
Size: 128 bytes
```

### Detection

If the first byte is `0xFF`, no melody is configured.

### Note Format

Each note is encoded in a single byte:

```
Bit layout: [NOTE:4][OCTAVE:2][DURATION:2]

┌─────────┬─────────┬─────────┐
│ NOTE    │ OCTAVE  │DURATION │
│ [7:4]   │ [3:2]   │ [1:0]   │
└─────────┴─────────┴─────────┘
```

### Note Values (4 bits)

| Value | Note |
|-------|------|
| 0 | C |
| 1 | C# |
| 2 | D |
| 3 | D# |
| 4 | E |
| 5 | F |
| 6 | F# |
| 7 | G |
| 8 | G# |
| 9 | A |
| 10 | A# |
| 11 | B |
| 12 | Pause |

### Octave (2 bits)

Relative to base octave 4:
- 0 = Octave 4
- 1 = Octave 5
- 2 = Octave 6
- 3 = Octave 7

### Duration (2 bits)

| Value | Duration |
|-------|----------|
| 0 | Whole note |
| 1 | Half note |
| 2 | Quarter note |
| 3 | Eighth note |

### Example

Byte `0x52` = `0101 00 10`:
- Note = 5 (F)
- Octave = 0 (octave 4)
- Duration = 2 (quarter note)

### Rust Implementation

```rust
// crates/esc-protocol/src/melody.rs

pub const MELODY_OFFSET: usize = 48;
pub const MELODY_SIZE: usize = 128;
pub const MELODY_EMPTY: u8 = 0xFF;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Note {
    C, Cs, D, Ds, E, F, Fs, G, Gs, A, As, B, Pause,
}

impl Note {
    pub fn from_nibble(n: u8) -> Self {
        match n & 0x0F {
            0 => Note::C, 1 => Note::Cs, 2 => Note::D, 3 => Note::Ds,
            4 => Note::E, 5 => Note::F, 6 => Note::Fs, 7 => Note::G,
            8 => Note::Gs, 9 => Note::A, 10 => Note::As, 11 => Note::B,
            _ => Note::Pause,
        }
    }

    pub fn to_nibble(self) -> u8 {
        match self {
            Note::C => 0, Note::Cs => 1, Note::D => 2, Note::Ds => 3,
            Note::E => 4, Note::F => 5, Note::Fs => 6, Note::G => 7,
            Note::Gs => 8, Note::A => 9, Note::As => 10, Note::B => 11,
            Note::Pause => 12,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Note::C => "C", Note::Cs => "C#", Note::D => "D", Note::Ds => "D#",
            Note::E => "E", Note::F => "F", Note::Fs => "F#", Note::G => "G",
            Note::Gs => "G#", Note::A => "A", Note::As => "A#", Note::B => "B",
            Note::Pause => "P",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MelodyNote {
    pub note: Note,
    pub octave: u8,      // 0-3 (add 4 for real octave)
    pub duration: u8,    // 0=whole, 1=half, 2=quarter, 3=eighth
}

impl MelodyNote {
    pub fn from_byte(b: u8) -> Self {
        Self {
            note: Note::from_nibble(b >> 4),
            octave: (b >> 2) & 0x03,
            duration: b & 0x03,
        }
    }

    pub fn to_byte(&self) -> u8 {
        (self.note.to_nibble() << 4) | (self.octave << 2) | self.duration
    }

    pub fn real_octave(&self) -> u8 {
        self.octave + 4
    }
}

pub struct Melody {
    pub notes: Vec<MelodyNote>,
}

impl Melody {
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.is_empty() || data[0] == MELODY_EMPTY {
            return None;
        }
        let notes = data.iter()
            .take_while(|&&b| b != 0x00 && b != 0xFF)
            .map(|&b| MelodyNote::from_byte(b))
            .collect();
        Some(Self { notes })
    }

    pub fn to_bytes(&self) -> [u8; MELODY_SIZE] {
        let mut buf = [0xFF; MELODY_SIZE];
        for (i, note) in self.notes.iter().take(MELODY_SIZE - 1).enumerate() {
            buf[i] = note.to_byte();
        }
        if !self.notes.is_empty() && self.notes.len() < MELODY_SIZE {
            buf[self.notes.len()] = 0x00;  // Terminator
        }
        buf
    }
}
```

### CLI Commands

```bash
# Download melody from ESC
esc-configurator melody download --device /dev/ttyACM0 --output melody.bin

# Upload melody to ESC
esc-configurator melody upload --device /dev/ttyACM0 --input melody.bin

# Clear melody
esc-configurator melody clear --device /dev/ttyACM0
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

## C++ Reference

From `widget.cpp`:

```cpp
// Music read
bool Widget::getMusic() {
    writeData(RL->setAddress(four_way->eeprom_address + 48));
    // ... wait for ACK ...
    writeData(RL->readFlash(128));
    // ... parse response ...

    if ((uint8_t)music.at(0) == 0xFF) {
        musicBufferFull = false;  // No melody
    } else {
        musicBufferFull = true;
        music_buffer->clear();
        for (int i = 0; i < 128; i++) {
            music_buffer->append(music[i]);
        }
    }
}

// Music write
bool Widget::writeMusic() {
    if (musicBufferFull) {
        QByteArray musicBufferOut;
        for (int i = 0; i < 128; i++) {
            musicBufferOut.append(music_buffer->at(i));
        }
        sendDirect(musicBufferOut, 128, four_way->eeprom_address + 48);
    }
}

// Presets
void Widget::sendFirstEeprom(uint8_t eeprom_type) {
    QByteArray eeprom_out;
    if (eeprom_type == 0) {
        for (int i = 0; i < 48; i++) {
            eeprom_out.append((char)air_starteeprom[i]);
        }
    }
    if (eeprom_type == 1) {
        for (int i = 0; i < 48; i++) {
            eeprom_out.append((char)crawler_starteeprom[i]);
        }
    }
    sendDirect(eeprom_out, 48, four_way->eeprom_address);
}

// Offline editing
void Widget::on_OfflineCheckBox_stateChanged(int arg1) {
    if (ui->OfflineCheckBox->isChecked()) {
        // Enable UI without connection
        firstOffline = true;
        on_loadConfigButton_clicked();  // Load defaults or file
    }
}
```
