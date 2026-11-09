# AM32 EEPROM Configuration Fields

Complete reference for the 48-byte AM32 configuration block.

---

## Complete Field Map

```
┌────────┬────────────────────────────┬───────┬─────────────────────────────┐
│ Offset │ Field                      │ Type  │ Notes                       │
├────────┼────────────────────────────┼───────┼─────────────────────────────┤
│ 0x00   │ boot_byte                  │ u8    │ 0x01=bootable, 0x00=stay BL │
│ 0x01   │ eeprom_version             │ u8    │ Config format version       │
│ 0x02   │ bootloader_version         │ u8    │ Bootloader version          │
│ 0x03   │ firmware_major             │ u8    │ Firmware major version      │
│ 0x04   │ firmware_minor             │ u8    │ Firmware minor version      │
├────────┼────────────────────────────┼───────┼─────────────────────────────┤
│ 0x05   │ max_rate_of_change         │ u8    │ 1-200, display ÷10          │
│ 0x06   │ min_duty                   │ u8    │ 0-50, display ÷2 %          │
│ 0x07   │ disable_stick_calibration  │ bool  │ 0=enabled, 1=disabled       │
│ 0x08   │ absolute_voltage           │ u8    │ Raw voltage value           │
│ 0x09   │ current_limit_p            │ u8    │ P gain, display ×2          │
│ 0x0A   │ current_limit_i            │ u8    │ I gain, raw                 │
│ 0x0B   │ current_limit_d            │ u8    │ D gain, display ×2          │
│ 0x0C   │ active_brake_strength      │ u8    │ 1-10                        │
├────────┼────────────────────────────┼───────┼─────────────────────────────┤
│ 0x0D   │ brake_on_zero_throttle     │ u8    │ Not exposed by the CLI yet  │
│ 0x0E   │ (reserved)                 │ -     │                             │
│ 0x0F   │ (reserved)                 │ -     │                             │
│ 0x10   │ (reserved)                 │ -     │                             │
├────────┼────────────────────────────┼───────┼─────────────────────────────┤
│ 0x11   │ direction_reversed         │ bool  │ Motor direction             │
│ 0x12   │ bidirectional              │ bool  │ 3D mode                     │
│ 0x13   │ sinusoidal_startup         │ bool  │ Sine startup mode           │
│ 0x14   │ complementary_pwm          │ bool  │ Comp PWM                    │
│ 0x15   │ variable_pwm               │ u8    │ 0=off, 1=var, 2=auto        │
│ 0x16   │ stuck_rotor_protection     │ bool  │ Stuck rotor detect          │
│ 0x17   │ timing_advance             │ u8    │ Raw value (see formula)     │
│ 0x18   │ pwm_frequency              │ u8    │ kHz (16-96)                 │
│ 0x19   │ startup_power              │ u8    │ 50-150 %                    │
│ 0x1A   │ motor_kv                   │ u8    │ ×40 = actual KV             │
│ 0x1B   │ motor_poles                │ u8    │ 2-36                        │
│ 0x1C   │ brake_on_stop              │ u8    │ 0=off, 1=brake, 2=active    │
│ 0x1D   │ stall_protection           │ bool  │ Anti-stall                  │
│ 0x1E   │ beep_volume                │ u8    │ 0-11                        │
│ 0x1F   │ telemetry_30ms             │ bool  │ 30ms telemetry output       │
│ 0x20   │ servo_low                  │ u8    │ ×2+750 = μs                 │
│ 0x21   │ servo_high                 │ u8    │ ×2+1750 = μs                │
│ 0x22   │ servo_neutral              │ u8    │ +1374 = μs                  │
│ 0x23   │ servo_deadband             │ u8    │ Raw deadband value          │
│ 0x24   │ low_voltage_cutoff         │ bool  │ LVC enable                  │
│ 0x25   │ lvc_threshold              │ u8    │ +250, then ÷100 = volts     │
│ 0x26   │ rc_car_reversing           │ bool  │ RC car mode                 │
│ 0x27   │ use_hall_sensors           │ bool  │ Hall sensor mode            │
│ 0x28   │ sine_mode_range            │ u8    │ 5-25 %                      │
│ 0x29   │ drag_brake_strength        │ u8    │ 1-10                        │
├────────┼────────────────────────────┼───────┼─────────────────────────────┤
│ 0x2A   │ running_brake_strength     │ u8    │ Running brake 1-10          │
│ 0x2B   │ temperature_limit          │ u8    │ 70-142°C, >140=disabled     │
│ 0x2C   │ current_limit              │ u8    │ ×2=Amps, >100=disabled      │
│ 0x2D   │ sine_mode_power            │ u8    │ 1-10                        │
│ 0x2E   │ signal_type                │ u8    │ Input signal type (enum)    │
│ 0x2F   │ auto_timing                │ bool  │ Auto timing adjust          │
└────────┴────────────────────────────┴───────┴─────────────────────────────┘
```

---

## Value Conversions

### Timing Advance (0x17)
```
Display value (degrees) = (raw_value - 10) × 0.9375
```
| Raw | Display |
|-----|---------|
| 10 | 0.0° |
| 18 | 7.5° |
| 26 | 15.0° |
| 34 | 22.5° |
| 42 | 30.0° |

### Motor KV (0x1A)
```
Actual KV = raw_value × 40 + 20
```
| Raw | KV |
|-----|-----|
| 0 | 20 |
| 22 | 900 |
| 55 | 2220 |

### Servo Low (0x20)
```
Microseconds = raw_value × 2 + 750
```
| Raw | μs |
|-----|-----|
| 0 | 750 |
| 128 | 1006 |
| 255 | 1260 |

### Servo High (0x21)
```
Microseconds = raw_value × 2 + 1750
```
| Raw | μs |
|-----|-----|
| 0 | 1750 |
| 128 | 2006 |
| 255 | 2260 |

### Servo Neutral (0x22)
```
Microseconds = raw_value + 1374
```

### LVC Threshold (0x25)
```
Volts = (raw_value + 250) / 100
```
| Raw | Volts |
|-----|-------|
| 0 | 2.5V |
| 100 | 3.5V |
| 200 | 4.5V |

---

## Enums

### Variable PWM Mode (0x15)
```rust
#[repr(u8)]
pub enum VariablePwmMode {
    Fixed = 0,
    Variable = 1,
    Auto = 2,
}
```

### Brake Mode (0x1C)
```rust
#[repr(u8)]
pub enum BrakeMode {
    Off = 0,
    Brake = 1,
    ActiveBrake = 2,
}
```

### Signal Type (0x2E)
```rust
#[repr(u8)]
pub enum SignalType {
    Auto = 0,
    DShot = 1,
    Servo = 2,
    // Values may vary - check AM32 source
}
```

---

## Rust Implementation

```rust
// crates/esc-protocol/src/am32_config.rs

// Add these constants
pub const OFFSET_BOOT_BYTE: usize = 0x00;
pub const OFFSET_EEPROM_VERSION: usize = 0x01;
pub const OFFSET_BOOTLOADER_VERSION: usize = 0x02;
pub const OFFSET_FIRMWARE_MAJOR: usize = 0x03;
pub const OFFSET_FIRMWARE_MINOR: usize = 0x04;
pub const OFFSET_MAX_RATE_OF_CHANGE: usize = 0x05;
pub const OFFSET_MIN_DUTY: usize = 0x06;
pub const OFFSET_DISABLE_STICK_CALIB: usize = 0x07;
pub const OFFSET_ABSOLUTE_VOLTAGE: usize = 0x08;
pub const OFFSET_CURRENT_LIMIT_P: usize = 0x09;
pub const OFFSET_CURRENT_LIMIT_I: usize = 0x0A;
pub const OFFSET_CURRENT_LIMIT_D: usize = 0x0B;
pub const OFFSET_ACTIVE_BRAKE_STRENGTH: usize = 0x0C;
// ... existing offsets ...
pub const OFFSET_RUNNING_BRAKE_STRENGTH: usize = 0x2A;
pub const OFFSET_TEMPERATURE_LIMIT: usize = 0x2B;
pub const OFFSET_CURRENT_LIMIT: usize = 0x2C;
pub const OFFSET_SINE_MODE_POWER: usize = 0x2D;
pub const OFFSET_SIGNAL_TYPE: usize = 0x2E;
pub const OFFSET_AUTO_TIMING: usize = 0x2F;

impl Am32Config {
    // Add these accessors:

    /// Max rate of change (1-200, display as /10.0)
    pub fn max_rate_of_change(&self) -> u8 {
        self.data[OFFSET_MAX_RATE_OF_CHANGE]
    }

    pub fn set_max_rate_of_change(&mut self, value: u8) {
        self.data[OFFSET_MAX_RATE_OF_CHANGE] = value.clamp(1, 200);
    }

    /// Max rate of change as display value
    pub fn max_rate_of_change_display(&self) -> f32 {
        self.max_rate_of_change() as f32 / 10.0
    }

    /// Min duty (0-50, display as /2.0 %)
    pub fn min_duty(&self) -> u8 {
        self.data[OFFSET_MIN_DUTY]
    }

    pub fn set_min_duty(&mut self, value: u8) {
        self.data[OFFSET_MIN_DUTY] = value.clamp(0, 50);
    }

    pub fn min_duty_percent(&self) -> f32 {
        self.min_duty() as f32 / 2.0
    }

    /// Disable stick calibration
    pub fn disable_stick_calibration(&self) -> bool {
        self.data[OFFSET_DISABLE_STICK_CALIB] != 0
    }

    pub fn set_disable_stick_calibration(&mut self, value: bool) {
        self.data[OFFSET_DISABLE_STICK_CALIB] = value as u8;
    }

    /// Current limit P gain (display as ×2)
    pub fn current_limit_p(&self) -> u8 {
        self.data[OFFSET_CURRENT_LIMIT_P]
    }

    pub fn current_limit_p_display(&self) -> u16 {
        self.current_limit_p() as u16 * 2
    }

    /// Temperature limit (70-142°C, >140 = disabled)
    pub fn temperature_limit(&self) -> u8 {
        self.data[OFFSET_TEMPERATURE_LIMIT]
    }

    pub fn temperature_limit_celsius(&self) -> Option<u8> {
        let val = self.temperature_limit();
        if val > 140 {
            None  // Disabled
        } else {
            Some(val)
        }
    }

    /// Current limit (×2 = Amps, >100 = disabled)
    pub fn current_limit(&self) -> u8 {
        self.data[OFFSET_CURRENT_LIMIT]
    }

    pub fn current_limit_amps(&self) -> Option<u16> {
        let val = self.current_limit();
        if val > 100 {
            None  // Disabled
        } else {
            Some(val as u16 * 2)
        }
    }

    /// Signal type
    pub fn signal_type(&self) -> SignalType {
        SignalType::from_byte(self.data[OFFSET_SIGNAL_TYPE])
    }

    /// Auto timing
    pub fn auto_timing(&self) -> bool {
        self.data[OFFSET_AUTO_TIMING] != 0
    }

    pub fn set_auto_timing(&mut self, value: bool) {
        self.data[OFFSET_AUTO_TIMING] = value as u8;
    }
}
```

---

## Settings by Category

### Motor Settings
- direction_reversed (0x11)
- bidirectional (0x12)
- motor_kv (0x1A)
- motor_poles (0x1B)
- timing_advance (0x17)
- auto_timing (0x2F)

### Startup Settings
- sinusoidal_startup (0x13)
- startup_power (0x19)
- sine_mode_range (0x28)
- sine_mode_power (0x2D)

### PWM Settings
- pwm_frequency (0x18)
- complementary_pwm (0x14)
- variable_pwm (0x15)

### Brake Settings
- brake_on_stop (0x1C)
- drag_brake_strength (0x29)
- active_brake_strength (0x0C)
- running_brake_strength (0x2A)

### Protection Settings
- stuck_rotor_protection (0x16)
- stall_protection (0x1D)
- temperature_limit (0x2B)
- current_limit (0x2C)
- low_voltage_cutoff (0x24)
- lvc_threshold (0x25)

### Input Settings
- signal_type (0x2E)
- servo_low (0x20)
- servo_high (0x21)
- servo_neutral (0x22)
- servo_deadband (0x23)
- disable_stick_calibration (0x07)
- rc_car_reversing (0x26)

### Other Settings
- beep_volume (0x1E)
- telemetry_30ms (0x1F)
- use_hall_sensors (0x27)

---

## C++ Reference

From `widget.cpp`:

```cpp
// Reading config into UI
ui->maxRocSlider->setValue((uint8_t)(input_buffer->at(5)));
ui->minDutySlider->setValue((uint8_t)(input_buffer->at(6)));
ui->disableStickCalibCheckbox->setChecked(input_buffer->at(7) == 0x01);
ui->absoluteVoltageSlider->setValue((uint8_t)(input_buffer->at(8)));
ui->currentLimitPedit->setText(QString::number((uint8_t)(input_buffer->at(9))*2));
ui->currentLimitIedit->setText(QString::number((uint8_t)(input_buffer->at(10))));
ui->currentLimitDedit->setText(QString::number((uint8_t)(input_buffer->at(11))*2));
ui->activeBrakeSlider->setValue((uint8_t)(input_buffer->at(12)));

// Timing advance display
ui->timingAdvanceLCD->display(((input_buffer->at(23))-10)*0.9375);

// Writing config from UI
eeprom_out[5] = ui->maxRocSlider->value();
eeprom_out[9] = (ui->currentLimitPedit->text().toInt())/2;
```
