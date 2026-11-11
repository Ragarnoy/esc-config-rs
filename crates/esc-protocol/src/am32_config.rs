//! AM32 ESC configuration parser.
//!
//! Parses the 256-byte EEPROM config block into structured fields.
//! Based on the Open ESC EEPROM Format from <https://wiki.am32.ca/development/Open-ESC-EEPROM-Format.html>
//!
//! Unlike BLHeli_S, AM32 config is NOT encrypted.

use core::fmt;

/// AM32 configuration block with parsed fields.
#[derive(Clone, Debug)]
pub struct Am32Config {
    /// Raw 256-byte config data
    raw: [u8; 256],
}

/// Motor timing advance, in AM32's units of 60/64 = 0.9375 electrical degrees
/// (0 to 32 steps, 0° to 30°).
///
/// AM32 stores it at `advance_level` in one of two encodings, and its
/// `loadEEpromSettings()` reads both:
///
/// - **old** (configurators before 1.90): `0..=3`, each step 7.5° (`raw << 3`);
/// - **new**: `10..=42`, `raw - 10` steps of 0.9375°.
///
/// Anything else (`4..=9`, `> 42`) runs as 16 steps (15°). [`MotorTiming::from_byte`]
/// decodes exactly that way. [`Am32Config::set_timing_advance`] keeps the encoding
/// the ESC already uses where the value fits it (see there).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct MotorTiming {
    steps: u8,
}

#[allow(non_upper_case_globals)]
impl MotorTiming {
    /// 0°.
    pub const Deg0: Self = Self { steps: 0 };
    /// 7.5°.
    pub const Deg7_5: Self = Self { steps: 8 };
    /// 15°.
    pub const Deg15: Self = Self { steps: 16 };
    /// 22.5°.
    pub const Deg22_5: Self = Self { steps: 24 };
    /// 30°.
    pub const Deg30: Self = Self { steps: 32 };

    /// Largest timing, in steps (30°).
    pub const MAX_STEPS: u8 = 32;
    /// Degrees per step.
    pub const DEGREES_PER_STEP: f32 = 60.0 / 64.0;
    /// What AM32 runs for an out-of-range byte.
    const FALLBACK_STEPS: u8 = 16;

    /// From steps of 0.9375°; `None` above [`Self::MAX_STEPS`].
    pub const fn from_steps(steps: u8) -> Option<Self> {
        if steps <= Self::MAX_STEPS {
            Some(Self { steps })
        } else {
            None
        }
    }

    /// Nearest step to `degrees`; `None` outside 0..=30.
    pub fn from_degrees(degrees: f32) -> Option<Self> {
        if !(0.0..=30.0).contains(&degrees) {
            return None;
        }
        // Round half up without `f32::round` (no_std).
        let steps = (degrees / Self::DEGREES_PER_STEP + 0.5) as u8;
        Self::from_steps(steps.min(Self::MAX_STEPS))
    }

    /// Decode a stored `advance_level` byte the way AM32's `loadEEpromSettings()`
    /// does.
    pub const fn from_byte(b: u8) -> Self {
        let steps = match b {
            0..=3 => b << 3,
            10..=42 => b - 10,
            _ => Self::FALLBACK_STEPS,
        };
        Self { steps }
    }

    /// The new (10..=42) encoding.
    pub const fn to_byte(self) -> u8 {
        self.steps + 10
    }

    /// The old (0..=3) encoding, if this value has one: 0°, 7.5°, 15° or 22.5°.
    pub const fn to_legacy_byte(self) -> Option<u8> {
        if self.steps.is_multiple_of(8) && self.steps <= 24 {
            Some(self.steps / 8)
        } else {
            None
        }
    }

    /// Steps of 0.9375°.
    pub const fn steps(self) -> u8 {
        self.steps
    }

    pub const fn degrees(self) -> f32 {
        self.steps as f32 * Self::DEGREES_PER_STEP
    }
}

impl fmt::Display for MotorTiming {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}°", self.degrees())
    }
}

/// Serialized as degrees (a number). Also reads the strings older versions of
/// this crate wrote (`"Deg15"`), except `"Deg30"`: that decoder reported 30° for
/// every byte of 4 and above, so an old `Deg30` export doesn't say what the ESC
/// had and is refused.
#[cfg(feature = "serde")]
impl serde::Serialize for MotorTiming {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_f32(self.degrees())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for MotorTiming {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = MotorTiming;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("timing advance in degrees (0-30)")
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<MotorTiming, E> {
                MotorTiming::from_degrees(v as f32)
                    .ok_or_else(|| E::custom("timing advance must be 0-30 degrees"))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<MotorTiming, E> {
                self.visit_f64(v as f64)
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<MotorTiming, E> {
                self.visit_f64(v as f64)
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<MotorTiming, E> {
                match v {
                    "Deg0" => Ok(MotorTiming::Deg0),
                    "Deg7_5" => Ok(MotorTiming::Deg7_5),
                    "Deg15" => Ok(MotorTiming::Deg15),
                    "Deg22_5" => Ok(MotorTiming::Deg22_5),
                    "Deg30" => Err(E::custom(
                        "legacy timing \"Deg30\" is ambiguous (older versions showed every \
                         stored value of 4 and above as 30°): give the timing in degrees",
                    )),
                    _ => Err(E::custom("expected timing advance in degrees (0-30)")),
                }
            }
        }
        d.deserialize_any(V)
    }
}

/// Variable PWM mode (offset 0x15)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum VariablePwmMode {
    /// Fixed PWM frequency
    #[default]
    Fixed = 0,
    /// Variable PWM frequency
    Variable = 1,
    /// Auto PWM frequency
    Auto = 2,
}

impl VariablePwmMode {
    pub const fn from_byte(b: u8) -> Self {
        match b {
            0 => Self::Fixed,
            1 => Self::Variable,
            _ => Self::Auto,
        }
    }
}

impl fmt::Display for VariablePwmMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fixed => write!(f, "Fixed"),
            Self::Variable => write!(f, "Variable"),
            Self::Auto => write!(f, "Auto"),
        }
    }
}

/// Brake mode (offset 0x1C)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum BrakeMode {
    /// No braking
    #[default]
    Off = 0,
    /// Normal brake
    Brake = 1,
    /// Active brake (regenerative)
    ActiveBrake = 2,
}

impl BrakeMode {
    pub const fn from_byte(b: u8) -> Self {
        match b {
            0 => Self::Off,
            1 => Self::Brake,
            _ => Self::ActiveBrake,
        }
    }
}

impl fmt::Display for BrakeMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Off => write!(f, "Off"),
            Self::Brake => write!(f, "Brake"),
            Self::ActiveBrake => write!(f, "Active Brake"),
        }
    }
}

/// Signal input type (offset 0x2E / 46)
/// Maps to AM32 inputType enum
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum SignalType {
    /// Auto-detect signal type
    #[default]
    Auto = 0,
    /// DShot digital protocol (unidirectional)
    DShot = 1,
    /// Servo/PWM analog signal
    Servo = 2,
    /// Serial protocol
    Serial = 3,
    /// Extended DShot Telemetry with arming (bidirectional)
    EdtArm = 4,
    /// DroneCAN protocol
    DroneCan = 5,
}

impl SignalType {
    pub const fn from_byte(b: u8) -> Self {
        match b {
            0 => Self::Auto,
            1 => Self::DShot,
            2 => Self::Servo,
            3 => Self::Serial,
            4 => Self::EdtArm,
            5 => Self::DroneCan,
            _ => Self::Auto,
        }
    }
}

impl fmt::Display for SignalType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto => write!(f, "Auto"),
            Self::DShot => write!(f, "DShot"),
            Self::Servo => write!(f, "Servo/PWM"),
            Self::Serial => write!(f, "Serial"),
            Self::EdtArm => write!(f, "EDT+Arm (Bidirectional)"),
            Self::DroneCan => write!(f, "DroneCAN"),
        }
    }
}

// EEPROM byte offsets for AM32 v2.20+
// Based on EEprom_t union from AM32 firmware
mod offsets {
    // Header (0x00-0x04)
    pub const RESERVED_0: usize = 0x00; // Boot byte: the bootloader jumps only if 0x01
    pub const EEPROM_VERSION: usize = 0x01;
    pub const RESERVED_1: usize = 0x02; // Was bootloader_version in older versions
    pub const FIRMWARE_MAJOR: usize = 0x03;
    pub const FIRMWARE_MINOR: usize = 0x04;

    // Throttle/Rate settings (0x05-0x0C)
    pub const MAX_RATE_OF_CHANGE: usize = 0x05; // max_ramp: 0.1% to 25% per ms
    pub const MIN_DUTY: usize = 0x06; // minimum_duty_cycle: 0.2% to 51%
    pub const DISABLE_STICK_CALIBRATION: usize = 0x07;
    pub const ABSOLUTE_VOLTAGE: usize = 0x08; // absolute_voltage_cutoff: 1-100 in 0.5V
    pub const CURRENT_LIMIT_P: usize = 0x09; // current_P: 0-255
    pub const CURRENT_LIMIT_I: usize = 0x0A; // current_I: 0-255
    pub const CURRENT_LIMIT_D: usize = 0x0B; // current_D: 0-255
    pub const ACTIVE_BRAKE_STRENGTH: usize = 0x0C; // active_brake_power: 1-5%
    #[allow(dead_code)]
    pub const BRAKE_ON_ZERO_THROTTLE: usize = 0x0D; // brake_on_zero_throttle (not exposed yet)

    // Reserved (0x0E-0x10)

    // Motor settings (0x11-0x1B)
    pub const DIRECTION_REVERSED: usize = 0x11; // dir_reversed
    pub const BIDIRECTIONAL: usize = 0x12; // bi_direction
    pub const SINUSOIDAL_STARTUP: usize = 0x13; // use_sine_start
    pub const COMPLEMENTARY_PWM: usize = 0x14; // comp_pwm
    pub const VARIABLE_PWM: usize = 0x15; // variable_pwm
    pub const STUCK_ROTOR_PROTECTION: usize = 0x16;
    pub const TIMING_ADVANCE: usize = 0x17; // advance_level
    pub const PWM_FREQUENCY: usize = 0x18;
    pub const STARTUP_POWER: usize = 0x19;
    pub const MOTOR_KV: usize = 0x1A;
    pub const MOTOR_POLES: usize = 0x1B;

    // Brake and protection (0x1C-0x1D)
    pub const BRAKE_ON_STOP: usize = 0x1C;
    pub const STALL_PROTECTION: usize = 0x1D;

    // Audio and telemetry (0x1E-0x1F)
    pub const BEEP_VOLUME: usize = 0x1E;
    pub const TELEMETRY_ON_INTERVAL: usize = 0x1F; // telemetry_on_interval

    // Servo/PWM input (0x20-0x23)
    pub const SERVO_LOW: usize = 0x20; // servo.low_threshold
    pub const SERVO_HIGH: usize = 0x21; // servo.high_threshold
    pub const SERVO_NEUTRAL: usize = 0x22; // servo.neutral
    pub const SERVO_DEADBAND: usize = 0x23; // servo.dead_band

    // Low voltage cutoff (0x24-0x25)
    pub const LOW_VOLTAGE_CUTOFF: usize = 0x24;
    pub const LVC_THRESHOLD: usize = 0x25; // low_cell_volt_cutoff

    // Input modes (0x26-0x27)
    pub const RC_CAR_REVERSING: usize = 0x26; // rc_car_reverse
    pub const USE_HALL_SENSORS: usize = 0x27;

    // Sine mode (0x28)
    pub const SINE_MODE_RANGE: usize = 0x28; // sine_mode_changeover_thottle_level

    // Brake strengths (0x29-0x2A)
    pub const DRAG_BRAKE_STRENGTH: usize = 0x29;
    pub const RUNNING_BRAKE_STRENGTH: usize = 0x2A; // driving_brake_strength

    // Limits (0x2B-0x2C)
    pub const TEMPERATURE_LIMIT: usize = 0x2B; // limits.temperature
    pub const CURRENT_LIMIT: usize = 0x2C; // limits.current

    // Additional settings (0x2D-0x2F)
    pub const SINE_MODE_POWER: usize = 0x2D;
    pub const INPUT_TYPE: usize = 0x2E; // input_type (was signal_type)
    pub const AUTO_TIMING: usize = 0x2F; // auto_advance

    // Tune/Melody (0x30-0xAF) - 128 bytes
    #[allow(dead_code)]
    pub const TUNE_START: usize = 0x30;
    #[allow(dead_code)]
    pub const TUNE_END: usize = 0xAF;

    // CAN settings (0xB0-0xBF)
    #[allow(dead_code)]
    pub const CAN_NODE: usize = 0xB0;
    #[allow(dead_code)]
    pub const CAN_ESC_INDEX: usize = 0xB1;
    #[allow(dead_code)]
    pub const CAN_REQUIRE_ARMING: usize = 0xB2;
    #[allow(dead_code)]
    pub const CAN_TELEM_RATE: usize = 0xB3;
    #[allow(dead_code)]
    pub const CAN_REQUIRE_ZERO_THROTTLE: usize = 0xB4;
    #[allow(dead_code)]
    pub const CAN_FILTER_HZ: usize = 0xB5;
    #[allow(dead_code)]
    pub const CAN_DEBUG_RATE: usize = 0xB6;
    #[allow(dead_code)]
    pub const CAN_TERM_ENABLE: usize = 0xB7;

    // Legacy aliases for backwards compatibility
    pub const BOOT_BYTE: usize = RESERVED_0;
    pub const BOOTLOADER_VERSION: usize = RESERVED_1;
    pub const SIGNAL_TYPE: usize = INPUT_TYPE;
    pub const TELEMETRY_30MS: usize = TELEMETRY_ON_INTERVAL;
}

impl Am32Config {
    /// Create config from raw 256-byte array.
    pub const fn from_bytes(raw: [u8; 256]) -> Self {
        Self { raw }
    }

    /// EEPROM layout version written by current AM32 firmware (`EEPROM_VERSION`).
    ///
    /// Firmware treats a lower version as an old layout and overwrites bytes 5-11
    /// with its own defaults on the next boot.
    pub const CURRENT_EEPROM_VERSION: u8 = 4;

    /// Create a config block holding only default values.
    ///
    /// Melody and CAN bytes are erased (0xFF) and the firmware version bytes are
    /// zero, so the firmware rewrites them on its next boot. Prefer reading the
    /// ESC's block and calling [`Self::apply_defaults`], which keeps them.
    pub fn with_defaults() -> Self {
        let mut config = Self::from_bytes([0xFF; 256]);
        config.raw[offsets::FIRMWARE_MAJOR] = 0;
        config.raw[offsets::FIRMWARE_MINOR] = 0;
        config.raw[offsets::RESERVED_1] = 0;
        for b in &mut config.raw[0x0E..=0x10] {
            *b = 0;
        }
        config.raw[offsets::BRAKE_ON_ZERO_THROTTLE] = 0;
        config.apply_defaults();
        config
    }

    /// Reset every setting to its default, keeping melody, CAN and version bytes.
    ///
    /// Also makes the ESC bootable (byte 0 = 0x01) and raises the EEPROM version
    /// to [`Self::CURRENT_EEPROM_VERSION`] if it is lower, so the firmware keeps
    /// these values instead of replacing them.
    pub fn apply_defaults(&mut self) {
        let config = self;
        config.set_boot_byte(Self::BOOT_BYTE_ENABLED);
        if config.raw[offsets::EEPROM_VERSION] < Self::CURRENT_EEPROM_VERSION
            || config.raw[offsets::EEPROM_VERSION] == 0xFF
        {
            config.raw[offsets::EEPROM_VERSION] = Self::CURRENT_EEPROM_VERSION;
        }

        // Throttle/Rate settings
        config.set_max_rate_of_change(100); // 10.0 display
        config.set_min_duty(10); // 5% display
        config.set_disable_stick_calibration(false);
        config.set_absolute_voltage(0);
        config.set_current_limit_p(50);
        config.set_current_limit_i(50);
        config.set_current_limit_d(50);
        config.set_active_brake_strength(5);

        // Motor settings
        config.set_direction_reversed(false);
        config.set_bidirectional(false);
        config.set_sinusoidal_startup(false);
        config.set_complementary_pwm(false);
        config.set_variable_pwm_mode(VariablePwmMode::Fixed);
        config.set_stuck_rotor_protection(true);
        config.set_timing_advance(MotorTiming::Deg22_5);
        config.set_auto_timing(true);
        config.set_pwm_frequency_khz(24);
        config.set_startup_power(100);
        config.set_motor_kv(2000);
        config.set_motor_poles(14);

        // Brake settings
        config.set_brake_mode(BrakeMode::Off);
        config.set_stall_protection(true);
        config.set_drag_brake_strength(1);
        config.set_running_brake_strength(1);

        // Audio/telemetry
        config.set_beep_volume(5);
        config.set_telemetry_30ms(false);

        // Servo input - these are only used in Servo mode
        config.set_servo_low(125); // 1000µs
        config.set_servo_high(125); // 2000µs
        config.set_servo_neutral(126); // 1500µs
        config.set_servo_deadband(50);

        // Signal type - CRITICAL: Auto to accept DShot
        config.set_signal_type(SignalType::Auto);

        // Limits
        config.set_low_voltage_cutoff(false);
        config.set_lvc_threshold(100); // 3.5V
        config.set_temperature_limit(142); // Disabled
        config.set_current_limit(101); // Disabled

        // Input modes
        config.set_rc_car_reversing(false);
        config.set_use_hall_sensors(false);

        // Sine mode
        config.set_sine_mode_range(5);
        config.set_sine_mode_power(5);
    }

    /// Get raw bytes.
    pub const fn as_bytes(&self) -> &[u8; 256] {
        &self.raw
    }

    /// Get mutable raw bytes.
    pub const fn as_bytes_mut(&mut self) -> &mut [u8; 256] {
        &mut self.raw
    }

    // =========================================================================
    // Safety Bit Handling
    // =========================================================================

    /// Boot byte value that enables ESC booting.
    pub const BOOT_BYTE_ENABLED: u8 = 0x01;

    /// Boot byte value that disables ESC booting (safe for flashing).
    pub const BOOT_BYTE_DISABLED: u8 = 0x00;

    /// ESC boot byte (must be 1 for ESC to boot).
    pub const fn boot_byte(&self) -> u8 {
        self.raw[offsets::BOOT_BYTE]
    }

    /// Set the boot byte to a specific value.
    pub const fn set_boot_byte(&mut self, value: u8) {
        self.raw[offsets::BOOT_BYTE] = value;
    }

    /// Check if the ESC is bootable (boot byte == 0x01).
    pub const fn is_bootable(&self) -> bool {
        self.boot_byte() == Self::BOOT_BYTE_ENABLED
    }

    /// Get the boot status for diagnostics.
    ///
    /// Returns a `BootStatus` enum indicating whether the ESC is bootable,
    /// stuck in bootloader, or has an unexpected boot byte value.
    pub const fn boot_status(&self) -> BootStatus {
        match self.boot_byte() {
            Self::BOOT_BYTE_ENABLED => BootStatus::Bootable,
            Self::BOOT_BYTE_DISABLED => BootStatus::StuckInBootloader,
            other => BootStatus::Unknown(other),
        }
    }

    /// Clear the boot byte (set to 0x00) to prevent booting.
    ///
    /// **SAFETY:** Call this before flashing firmware to ensure the ESC
    /// won't try to boot corrupted firmware if the flash fails.
    ///
    /// Returns the previous boot byte value for restoration.
    pub const fn clear_boot_byte(&mut self) -> u8 {
        let previous = self.boot_byte();
        self.raw[offsets::BOOT_BYTE] = Self::BOOT_BYTE_DISABLED;
        previous
    }

    /// Restore the boot byte (set to 0x01) to enable booting.
    ///
    /// **SAFETY:** Call this after successful firmware flash to allow
    /// the ESC to boot.
    pub const fn restore_boot_byte(&mut self) {
        self.raw[offsets::BOOT_BYTE] = Self::BOOT_BYTE_ENABLED;
    }

    /// Get the first 48 bytes of the EEPROM (the configurable region).
    ///
    /// This is the region that contains the boot byte and all config settings.
    /// Used for safety bit handling during firmware flashing.
    pub fn eeprom_bytes(&self) -> &[u8] {
        &self.raw[0..48]
    }

    /// Get a mutable reference to the first 48 bytes of the EEPROM.
    pub fn eeprom_bytes_mut(&mut self) -> &mut [u8] {
        &mut self.raw[0..48]
    }

    // =========================================================================
    // Read-only getters
    // =========================================================================

    /// EEPROM format version.
    pub const fn eeprom_version(&self) -> u8 {
        self.raw[offsets::EEPROM_VERSION]
    }

    /// Bootloader version.
    pub const fn bootloader_version(&self) -> u8 {
        self.raw[offsets::BOOTLOADER_VERSION]
    }

    /// Firmware version as (major, minor).
    pub const fn firmware_version(&self) -> (u8, u8) {
        (
            self.raw[offsets::FIRMWARE_MAJOR],
            self.raw[offsets::FIRMWARE_MINOR],
        )
    }

    // =========================================================================
    // Throttle/Rate settings (0x05-0x0C)
    // =========================================================================

    /// Max rate of change (1-200, display value = raw / 10.0).
    pub const fn max_rate_of_change(&self) -> u8 {
        self.raw[offsets::MAX_RATE_OF_CHANGE]
    }

    /// Max rate of change as display value.
    pub fn max_rate_of_change_display(&self) -> f32 {
        self.max_rate_of_change() as f32 / 10.0
    }

    pub fn set_max_rate_of_change(&mut self, value: u8) {
        self.raw[offsets::MAX_RATE_OF_CHANGE] = value.clamp(1, 200);
    }

    /// Min duty cycle (0-50, display value = raw / 2.0 %).
    pub const fn min_duty(&self) -> u8 {
        self.raw[offsets::MIN_DUTY]
    }

    /// Min duty as percentage.
    pub fn min_duty_percent(&self) -> f32 {
        self.min_duty() as f32 / 2.0
    }

    pub fn set_min_duty(&mut self, value: u8) {
        self.raw[offsets::MIN_DUTY] = value.clamp(0, 50);
    }

    /// Disable stick calibration (true = disabled).
    pub const fn disable_stick_calibration(&self) -> bool {
        self.raw[offsets::DISABLE_STICK_CALIBRATION] != 0
    }

    pub const fn set_disable_stick_calibration(&mut self, value: bool) {
        self.raw[offsets::DISABLE_STICK_CALIBRATION] = value as u8;
    }

    /// Absolute voltage reading (raw ADC value).
    pub const fn absolute_voltage(&self) -> u8 {
        self.raw[offsets::ABSOLUTE_VOLTAGE]
    }

    pub const fn set_absolute_voltage(&mut self, value: u8) {
        self.raw[offsets::ABSOLUTE_VOLTAGE] = value;
    }

    /// Current limit P gain (display value = raw * 2).
    pub const fn current_limit_p(&self) -> u8 {
        self.raw[offsets::CURRENT_LIMIT_P]
    }

    pub const fn current_limit_p_display(&self) -> u16 {
        self.current_limit_p() as u16 * 2
    }

    pub const fn set_current_limit_p(&mut self, value: u8) {
        self.raw[offsets::CURRENT_LIMIT_P] = value;
    }

    /// Current limit I gain (raw value).
    pub const fn current_limit_i(&self) -> u8 {
        self.raw[offsets::CURRENT_LIMIT_I]
    }

    pub const fn set_current_limit_i(&mut self, value: u8) {
        self.raw[offsets::CURRENT_LIMIT_I] = value;
    }

    /// Current limit D gain (display value = raw * 2).
    pub const fn current_limit_d(&self) -> u8 {
        self.raw[offsets::CURRENT_LIMIT_D]
    }

    pub const fn current_limit_d_display(&self) -> u16 {
        self.current_limit_d() as u16 * 2
    }

    pub const fn set_current_limit_d(&mut self, value: u8) {
        self.raw[offsets::CURRENT_LIMIT_D] = value;
    }

    /// Active brake strength (1-10).
    pub const fn active_brake_strength(&self) -> u8 {
        self.raw[offsets::ACTIVE_BRAKE_STRENGTH]
    }

    pub fn set_active_brake_strength(&mut self, value: u8) {
        self.raw[offsets::ACTIVE_BRAKE_STRENGTH] = value.clamp(1, 10);
    }

    // =========================================================================
    // Configurable settings with getters and setters
    // =========================================================================

    /// Motor direction reversed.
    pub const fn direction_reversed(&self) -> bool {
        self.raw[offsets::DIRECTION_REVERSED] != 0
    }

    pub const fn set_direction_reversed(&mut self, value: bool) {
        self.raw[offsets::DIRECTION_REVERSED] = value as u8;
    }

    /// Bidirectional mode (3D mode).
    pub const fn bidirectional(&self) -> bool {
        self.raw[offsets::BIDIRECTIONAL] != 0
    }

    pub const fn set_bidirectional(&mut self, value: bool) {
        self.raw[offsets::BIDIRECTIONAL] = value as u8;
    }

    /// Sinusoidal startup mode.
    pub const fn sinusoidal_startup(&self) -> bool {
        self.raw[offsets::SINUSOIDAL_STARTUP] != 0
    }

    pub const fn set_sinusoidal_startup(&mut self, value: bool) {
        self.raw[offsets::SINUSOIDAL_STARTUP] = value as u8;
    }

    /// Complementary PWM.
    pub const fn complementary_pwm(&self) -> bool {
        self.raw[offsets::COMPLEMENTARY_PWM] != 0
    }

    pub const fn set_complementary_pwm(&mut self, value: bool) {
        self.raw[offsets::COMPLEMENTARY_PWM] = value as u8;
    }

    /// Variable PWM mode.
    pub const fn variable_pwm_mode(&self) -> VariablePwmMode {
        VariablePwmMode::from_byte(self.raw[offsets::VARIABLE_PWM])
    }

    pub const fn set_variable_pwm_mode(&mut self, mode: VariablePwmMode) {
        self.raw[offsets::VARIABLE_PWM] = mode as u8;
    }

    /// Variable PWM enabled (legacy bool accessor).
    pub const fn variable_pwm(&self) -> bool {
        self.raw[offsets::VARIABLE_PWM] != 0
    }

    pub const fn set_variable_pwm(&mut self, value: bool) {
        self.raw[offsets::VARIABLE_PWM] = value as u8;
    }

    /// Stuck rotor protection.
    pub const fn stuck_rotor_protection(&self) -> bool {
        self.raw[offsets::STUCK_ROTOR_PROTECTION] != 0
    }

    pub const fn set_stuck_rotor_protection(&mut self, value: bool) {
        self.raw[offsets::STUCK_ROTOR_PROTECTION] = value as u8;
    }

    /// Motor timing advance.
    pub const fn timing_advance(&self) -> MotorTiming {
        MotorTiming::from_byte(self.raw[offsets::TIMING_ADVANCE])
    }

    /// Keeps the stored encoding where it can: a byte in the old `0..=3` form
    /// stays in that form when the new value has one (0°, 7.5°, 15°, 22.5°), so
    /// firmware that only knows the old form still reads it. Anything else is
    /// written in the new `10..=42` form.
    pub const fn set_timing_advance(&mut self, timing: MotorTiming) {
        let current = self.raw[offsets::TIMING_ADVANCE];
        self.raw[offsets::TIMING_ADVANCE] = match (current, timing.to_legacy_byte()) {
            (0..=3, Some(legacy)) => legacy,
            _ => timing.to_byte(),
        };
    }

    /// PWM frequency in kHz.
    pub const fn pwm_frequency_khz(&self) -> u8 {
        self.raw[offsets::PWM_FREQUENCY]
    }

    pub const fn set_pwm_frequency_khz(&mut self, khz: u8) {
        self.raw[offsets::PWM_FREQUENCY] = khz;
    }

    /// Startup power (50-150%).
    pub const fn startup_power(&self) -> u8 {
        self.raw[offsets::STARTUP_POWER]
    }

    pub fn set_startup_power(&mut self, power: u8) {
        self.raw[offsets::STARTUP_POWER] = power.clamp(50, 150);
    }

    /// Motor KV (raw value * 40 = actual KV).
    pub const fn motor_kv_raw(&self) -> u8 {
        self.raw[offsets::MOTOR_KV]
    }

    /// Motor KV in actual value.
    pub const fn motor_kv(&self) -> u16 {
        self.raw[offsets::MOTOR_KV] as u16 * 40
    }

    pub fn set_motor_kv(&mut self, kv: u16) {
        self.raw[offsets::MOTOR_KV] = (kv / 40).min(255) as u8;
    }

    /// Motor poles.
    pub const fn motor_poles(&self) -> u8 {
        self.raw[offsets::MOTOR_POLES]
    }

    pub const fn set_motor_poles(&mut self, poles: u8) {
        self.raw[offsets::MOTOR_POLES] = poles;
    }

    /// Brake mode on stop.
    pub const fn brake_mode(&self) -> BrakeMode {
        BrakeMode::from_byte(self.raw[offsets::BRAKE_ON_STOP])
    }

    pub const fn set_brake_mode(&mut self, mode: BrakeMode) {
        self.raw[offsets::BRAKE_ON_STOP] = mode as u8;
    }

    /// Brake on stop enabled (legacy bool accessor).
    pub const fn brake_on_stop(&self) -> bool {
        self.raw[offsets::BRAKE_ON_STOP] != 0
    }

    pub const fn set_brake_on_stop(&mut self, value: bool) {
        self.raw[offsets::BRAKE_ON_STOP] = value as u8;
    }

    /// Stall protection.
    pub const fn stall_protection(&self) -> bool {
        self.raw[offsets::STALL_PROTECTION] != 0
    }

    pub const fn set_stall_protection(&mut self, value: bool) {
        self.raw[offsets::STALL_PROTECTION] = value as u8;
    }

    /// Beep volume (0-11).
    pub const fn beep_volume(&self) -> u8 {
        self.raw[offsets::BEEP_VOLUME]
    }

    pub fn set_beep_volume(&mut self, volume: u8) {
        self.raw[offsets::BEEP_VOLUME] = volume.min(11);
    }

    /// 30ms telemetry output.
    pub const fn telemetry_30ms(&self) -> bool {
        self.raw[offsets::TELEMETRY_30MS] != 0
    }

    pub const fn set_telemetry_30ms(&mut self, value: bool) {
        self.raw[offsets::TELEMETRY_30MS] = value as u8;
    }

    /// Servo low value. Actual microseconds = (value * 2) + 750.
    pub const fn servo_low(&self) -> u8 {
        self.raw[offsets::SERVO_LOW]
    }

    pub const fn servo_low_us(&self) -> u16 {
        self.raw[offsets::SERVO_LOW] as u16 * 2 + 750
    }

    pub const fn set_servo_low(&mut self, value: u8) {
        self.raw[offsets::SERVO_LOW] = value;
    }

    /// Servo high value. Actual microseconds = (value * 2) + 1750.
    pub const fn servo_high(&self) -> u8 {
        self.raw[offsets::SERVO_HIGH]
    }

    pub const fn servo_high_us(&self) -> u16 {
        self.raw[offsets::SERVO_HIGH] as u16 * 2 + 1750
    }

    pub const fn set_servo_high(&mut self, value: u8) {
        self.raw[offsets::SERVO_HIGH] = value;
    }

    /// Servo neutral. Actual microseconds = 1374 + value.
    pub const fn servo_neutral(&self) -> u8 {
        self.raw[offsets::SERVO_NEUTRAL]
    }

    pub const fn servo_neutral_us(&self) -> u16 {
        1374 + self.raw[offsets::SERVO_NEUTRAL] as u16
    }

    pub const fn set_servo_neutral(&mut self, value: u8) {
        self.raw[offsets::SERVO_NEUTRAL] = value;
    }

    /// Servo deadband (raw value).
    pub const fn servo_deadband(&self) -> u8 {
        self.raw[offsets::SERVO_DEADBAND]
    }

    pub const fn set_servo_deadband(&mut self, value: u8) {
        self.raw[offsets::SERVO_DEADBAND] = value;
    }

    /// Low voltage cutoff enabled.
    pub const fn low_voltage_cutoff(&self) -> bool {
        self.raw[offsets::LOW_VOLTAGE_CUTOFF] != 0
    }

    pub const fn set_low_voltage_cutoff(&mut self, value: bool) {
        self.raw[offsets::LOW_VOLTAGE_CUTOFF] = value as u8;
    }

    /// Low voltage cutoff threshold (raw value).
    /// Actual voltage = (raw + 250) / 100.0
    pub const fn lvc_threshold(&self) -> u8 {
        self.raw[offsets::LVC_THRESHOLD]
    }

    /// LVC threshold in volts.
    pub fn lvc_threshold_volts(&self) -> f32 {
        (self.raw[offsets::LVC_THRESHOLD] as f32 + 250.0) / 100.0
    }

    pub const fn set_lvc_threshold(&mut self, value: u8) {
        self.raw[offsets::LVC_THRESHOLD] = value;
    }

    /// RC car style reversing.
    pub const fn rc_car_reversing(&self) -> bool {
        self.raw[offsets::RC_CAR_REVERSING] != 0
    }

    pub const fn set_rc_car_reversing(&mut self, value: bool) {
        self.raw[offsets::RC_CAR_REVERSING] = value as u8;
    }

    /// Use Hall sensors (if equipped).
    pub const fn use_hall_sensors(&self) -> bool {
        self.raw[offsets::USE_HALL_SENSORS] != 0
    }

    pub const fn set_use_hall_sensors(&mut self, value: bool) {
        self.raw[offsets::USE_HALL_SENSORS] = value as u8;
    }

    /// Sine mode range (5-25% throttle).
    pub const fn sine_mode_range(&self) -> u8 {
        self.raw[offsets::SINE_MODE_RANGE]
    }

    pub fn set_sine_mode_range(&mut self, value: u8) {
        self.raw[offsets::SINE_MODE_RANGE] = value.clamp(5, 25);
    }

    /// Drag brake strength (1-10).
    pub const fn drag_brake_strength(&self) -> u8 {
        self.raw[offsets::DRAG_BRAKE_STRENGTH]
    }

    pub fn set_drag_brake_strength(&mut self, value: u8) {
        self.raw[offsets::DRAG_BRAKE_STRENGTH] = value.clamp(1, 10);
    }

    /// Running brake strength (1-10).
    pub const fn running_brake_strength(&self) -> u8 {
        self.raw[offsets::RUNNING_BRAKE_STRENGTH]
    }

    pub fn set_running_brake_strength(&mut self, value: u8) {
        self.raw[offsets::RUNNING_BRAKE_STRENGTH] = value.clamp(1, 10);
    }

    /// Temperature limit (70-142°C, >140 = disabled).
    pub const fn temperature_limit(&self) -> u8 {
        self.raw[offsets::TEMPERATURE_LIMIT]
    }

    /// Temperature limit in Celsius, or None if disabled.
    pub const fn temperature_limit_celsius(&self) -> Option<u8> {
        let val = self.temperature_limit();
        if val > 140 {
            None // Disabled
        } else {
            Some(val)
        }
    }

    pub const fn set_temperature_limit(&mut self, value: u8) {
        self.raw[offsets::TEMPERATURE_LIMIT] = value;
    }

    /// Current limit (raw value, ×2 = Amps, >100 = disabled).
    pub const fn current_limit(&self) -> u8 {
        self.raw[offsets::CURRENT_LIMIT]
    }

    /// Current limit in Amps, or None if disabled.
    pub const fn current_limit_amps(&self) -> Option<u16> {
        let val = self.current_limit();
        if val > 100 {
            None // Disabled
        } else {
            Some(val as u16 * 2)
        }
    }

    pub const fn set_current_limit(&mut self, value: u8) {
        self.raw[offsets::CURRENT_LIMIT] = value;
    }

    /// Sine mode power (1-10).
    pub const fn sine_mode_power(&self) -> u8 {
        self.raw[offsets::SINE_MODE_POWER]
    }

    pub fn set_sine_mode_power(&mut self, value: u8) {
        self.raw[offsets::SINE_MODE_POWER] = value.clamp(1, 10);
    }

    /// Signal input type.
    pub const fn signal_type(&self) -> SignalType {
        SignalType::from_byte(self.raw[offsets::SIGNAL_TYPE])
    }

    pub const fn set_signal_type(&mut self, signal_type: SignalType) {
        self.raw[offsets::SIGNAL_TYPE] = signal_type as u8;
    }

    /// Auto timing enabled.
    pub const fn auto_timing(&self) -> bool {
        self.raw[offsets::AUTO_TIMING] != 0
    }

    pub const fn set_auto_timing(&mut self, value: bool) {
        self.raw[offsets::AUTO_TIMING] = value as u8;
    }
}

impl fmt::Display for Am32Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (fw_major, fw_minor) = self.firmware_version();
        writeln!(f, "AM32 ESC Configuration")?;
        writeln!(f, "======================")?;
        writeln!(f)?;
        writeln!(f, "Firmware:            v{}.{}", fw_major, fw_minor)?;
        writeln!(f, "Bootloader:          v{}", self.bootloader_version())?;
        writeln!(f, "EEPROM Version:      {}", self.eeprom_version())?;
        writeln!(f)?;

        writeln!(f, "Throttle/Rate Settings")?;
        writeln!(f, "----------------------")?;
        writeln!(
            f,
            "Max Rate of Change:  {} (display: {:.1})",
            self.max_rate_of_change(),
            self.max_rate_of_change_display()
        )?;
        writeln!(
            f,
            "Min Duty:            {} (display: {:.1}%)",
            self.min_duty(),
            self.min_duty_percent()
        )?;
        writeln!(
            f,
            "Stick Calib Disabled:{}",
            if self.disable_stick_calibration() {
                " Yes"
            } else {
                " No"
            }
        )?;
        writeln!(f)?;

        writeln!(f, "Current Limit PID")?;
        writeln!(f, "-----------------")?;
        writeln!(
            f,
            "P Gain:              {} (display: {})",
            self.current_limit_p(),
            self.current_limit_p_display()
        )?;
        writeln!(f, "I Gain:              {}", self.current_limit_i())?;
        writeln!(
            f,
            "D Gain:              {} (display: {})",
            self.current_limit_d(),
            self.current_limit_d_display()
        )?;
        writeln!(f)?;

        writeln!(f, "Motor Settings")?;
        writeln!(f, "--------------")?;
        writeln!(
            f,
            "Direction Reversed:  {}",
            if self.direction_reversed() {
                "Yes"
            } else {
                "No"
            }
        )?;
        writeln!(
            f,
            "Bidirectional (3D):  {}",
            if self.bidirectional() { "Yes" } else { "No" }
        )?;
        writeln!(
            f,
            "Motor KV:            {} (raw: {})",
            self.motor_kv(),
            self.motor_kv_raw()
        )?;
        writeln!(f, "Motor Poles:         {}", self.motor_poles())?;
        writeln!(f, "Timing Advance:      {}", self.timing_advance())?;
        writeln!(
            f,
            "Auto Timing:         {}",
            if self.auto_timing() { "Yes" } else { "No" }
        )?;
        writeln!(f)?;

        writeln!(f, "PWM Settings")?;
        writeln!(f, "------------")?;
        writeln!(f, "PWM Frequency:       {} kHz", self.pwm_frequency_khz())?;
        writeln!(
            f,
            "Complementary PWM:   {}",
            if self.complementary_pwm() {
                "Yes"
            } else {
                "No"
            }
        )?;
        writeln!(f, "Variable PWM:        {}", self.variable_pwm_mode())?;
        writeln!(f)?;

        writeln!(f, "Startup & Protection")?;
        writeln!(f, "--------------------")?;
        writeln!(f, "Startup Power:       {}%", self.startup_power())?;
        writeln!(
            f,
            "Sinusoidal Startup:  {}",
            if self.sinusoidal_startup() {
                "Yes"
            } else {
                "No"
            }
        )?;
        writeln!(
            f,
            "Stuck Rotor Prot.:   {}",
            if self.stuck_rotor_protection() {
                "Yes"
            } else {
                "No"
            }
        )?;
        writeln!(
            f,
            "Stall Protection:    {}",
            if self.stall_protection() { "Yes" } else { "No" }
        )?;
        writeln!(f)?;

        writeln!(f, "Braking")?;
        writeln!(f, "-------")?;
        writeln!(f, "Brake Mode:          {}", self.brake_mode())?;
        writeln!(
            f,
            "Active Brake:        {}/10",
            self.active_brake_strength()
        )?;
        writeln!(f, "Drag Brake:          {}/10", self.drag_brake_strength())?;
        writeln!(
            f,
            "Running Brake:       {}/10",
            self.running_brake_strength()
        )?;
        writeln!(f)?;

        writeln!(f, "Limits")?;
        writeln!(f, "------")?;
        writeln!(
            f,
            "LVC Enabled:         {}",
            if self.low_voltage_cutoff() {
                "Yes"
            } else {
                "No"
            }
        )?;
        writeln!(
            f,
            "LVC Threshold:       {:.2}V (raw: {})",
            self.lvc_threshold_volts(),
            self.lvc_threshold()
        )?;
        match self.temperature_limit_celsius() {
            Some(t) => writeln!(f, "Temperature Limit:   {}°C", t)?,
            None => writeln!(f, "Temperature Limit:   Disabled")?,
        }
        match self.current_limit_amps() {
            Some(a) => writeln!(f, "Current Limit:       {}A", a)?,
            None => writeln!(f, "Current Limit:       Disabled")?,
        }
        writeln!(f)?;

        writeln!(f, "Servo / PWM Input")?;
        writeln!(f, "-----------------")?;
        writeln!(f, "Signal Type:         {}", self.signal_type())?;
        writeln!(
            f,
            "Servo Low:           {} μs (raw: {})",
            self.servo_low_us(),
            self.servo_low()
        )?;
        writeln!(
            f,
            "Servo High:          {} μs (raw: {})",
            self.servo_high_us(),
            self.servo_high()
        )?;
        writeln!(
            f,
            "Servo Neutral:       {} μs (raw: {})",
            self.servo_neutral_us(),
            self.servo_neutral()
        )?;
        writeln!(f, "Servo Deadband:      {}", self.servo_deadband())?;
        writeln!(f)?;

        writeln!(f, "Sine Mode")?;
        writeln!(f, "---------")?;
        writeln!(f, "Sine Mode Range:     {}%", self.sine_mode_range())?;
        writeln!(f, "Sine Mode Power:     {}/10", self.sine_mode_power())?;
        writeln!(f)?;

        writeln!(f, "Other")?;
        writeln!(f, "-----")?;
        writeln!(f, "Beep Volume:         {}/11", self.beep_volume())?;
        writeln!(
            f,
            "30ms Telemetry:      {}",
            if self.telemetry_30ms() { "Yes" } else { "No" }
        )?;
        writeln!(
            f,
            "RC Car Reversing:    {}",
            if self.rc_car_reversing() { "Yes" } else { "No" }
        )?;
        writeln!(
            f,
            "Use Hall Sensors:    {}",
            if self.use_hall_sensors() { "Yes" } else { "No" }
        )?;
        Ok(())
    }
}

/// Setting name for CLI commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    // Throttle/Rate settings
    MaxRateOfChange,
    MinDuty,
    DisableStickCalibration,
    AbsoluteVoltage,
    CurrentLimitP,
    CurrentLimitI,
    CurrentLimitD,
    ActiveBrakeStrength,
    // Motor settings
    DirectionReversed,
    Bidirectional,
    SinusoidalStartup,
    ComplementaryPwm,
    VariablePwm,
    StuckRotorProtection,
    TimingAdvance,
    AutoTiming,
    PwmFrequency,
    StartupPower,
    MotorKv,
    MotorPoles,
    // Brake settings
    BrakeOnStop,
    StallProtection,
    DragBrakeStrength,
    RunningBrakeStrength,
    // Audio/telemetry
    BeepVolume,
    Telemetry30ms,
    // Servo input
    ServoLow,
    ServoHigh,
    ServoNeutral,
    ServoDeadband,
    // Voltage/current limits
    LowVoltageCutoff,
    LvcThreshold,
    TemperatureLimit,
    CurrentLimit,
    // Input modes
    RcCarReversing,
    UseHallSensors,
    SignalType,
    // Sine mode
    SineModeRange,
    SineModePower,
}

impl Setting {
    /// Parse setting name from string.
    pub fn parse(s: &str) -> Option<Self> {
        // Lowercase ASCII and drop '-' / '_' without allocating.
        let mut buf = [0u8; 48];
        let mut len = 0;
        for b in s.bytes().filter(|b| !matches!(b, b'-' | b'_')) {
            *buf.get_mut(len)? = b.to_ascii_lowercase();
            len += 1;
        }
        match core::str::from_utf8(&buf[..len]).ok()? {
            // Throttle/Rate settings
            "maxrateofchange" | "maxrate" | "roc" => Some(Self::MaxRateOfChange),
            "minduty" => Some(Self::MinDuty),
            "disablestickcalibration" | "disablestickcalib" | "stickcalib" => {
                Some(Self::DisableStickCalibration)
            }
            "absolutevoltage" | "absvoltage" => Some(Self::AbsoluteVoltage),
            "currentlimitp" | "currentp" | "clp" => Some(Self::CurrentLimitP),
            "currentlimiti" | "currenti" | "cli" => Some(Self::CurrentLimitI),
            "currentlimitd" | "currentd" | "cld" => Some(Self::CurrentLimitD),
            "activebrakestrength" | "activebrake" => Some(Self::ActiveBrakeStrength),
            // Motor settings
            "directionreversed" | "direction" | "reversed" | "dir" => Some(Self::DirectionReversed),
            "bidirectional" | "bidir" | "3d" | "3dmode" => Some(Self::Bidirectional),
            "sinusoidalstartup" | "sinusoidal" | "sinestart" => Some(Self::SinusoidalStartup),
            "complementarypwm" | "comppwm" => Some(Self::ComplementaryPwm),
            "variablepwm" | "varpwm" => Some(Self::VariablePwm),
            "stuckrotorprotection" | "stuckrotor" => Some(Self::StuckRotorProtection),
            "timingadvance" | "timing" => Some(Self::TimingAdvance),
            "autotiming" => Some(Self::AutoTiming),
            "pwmfrequency" | "pwmfreq" | "pwm" => Some(Self::PwmFrequency),
            "startuppower" | "startup" => Some(Self::StartupPower),
            "motorkv" | "kv" => Some(Self::MotorKv),
            "motorpoles" | "poles" => Some(Self::MotorPoles),
            // Brake settings
            "brakeonstop" | "brake" | "brakemode" => Some(Self::BrakeOnStop),
            "stallprotection" | "stall" => Some(Self::StallProtection),
            "dragbrakestrength" | "dragbrake" => Some(Self::DragBrakeStrength),
            "runningbrakestrength" | "runningbrake" => Some(Self::RunningBrakeStrength),
            // Audio/telemetry
            "beepvolume" | "beep" | "volume" => Some(Self::BeepVolume),
            "telemetry30ms" | "telemetry" | "telem" => Some(Self::Telemetry30ms),
            // Servo input
            "servolow" => Some(Self::ServoLow),
            "servohigh" => Some(Self::ServoHigh),
            "servoneutral" | "servocenter" => Some(Self::ServoNeutral),
            "servodeadband" | "deadband" => Some(Self::ServoDeadband),
            // Voltage/current limits
            "lowvoltagecutoff" | "lvc" => Some(Self::LowVoltageCutoff),
            "lvcthreshold" | "lvcvolts" => Some(Self::LvcThreshold),
            "temperaturelimit" | "templimit" | "temp" => Some(Self::TemperatureLimit),
            "currentlimit" | "current" => Some(Self::CurrentLimit),
            // Input modes
            "rccarreversing" | "rccar" => Some(Self::RcCarReversing),
            "usehallsensors" | "hall" => Some(Self::UseHallSensors),
            "signaltype" | "signal" => Some(Self::SignalType),
            // Sine mode
            "sinemoderange" | "sinerange" => Some(Self::SineModeRange),
            "sinemodepower" | "sinepower" => Some(Self::SineModePower),
            _ => None,
        }
    }

    /// Get all available setting names.
    pub const fn all_names() -> &'static [&'static str] {
        &[
            // Throttle/Rate
            "max-rate",
            "min-duty",
            "stick-calib",
            "abs-voltage",
            "current-p",
            "current-i",
            "current-d",
            "active-brake",
            // Motor
            "direction",
            "bidirectional",
            "sinusoidal",
            "comp-pwm",
            "variable-pwm",
            "stuck-rotor",
            "timing",
            "auto-timing",
            "pwm-freq",
            "startup-power",
            "motor-kv",
            "motor-poles",
            // Brake
            "brake",
            "stall",
            "drag-brake",
            "running-brake",
            // Audio
            "beep-volume",
            "telemetry",
            // Servo
            "servo-low",
            "servo-high",
            "servo-neutral",
            "deadband",
            // Limits
            "lvc",
            "lvc-threshold",
            "temp-limit",
            "current-limit",
            // Input modes
            "rc-car",
            "hall",
            "signal",
            // Sine mode
            "sine-range",
            "sine-power",
        ]
    }

    /// Returns true if this setting takes a boolean value.
    pub const fn is_bool(&self) -> bool {
        matches!(
            self,
            Self::DisableStickCalibration
                | Self::DirectionReversed
                | Self::Bidirectional
                | Self::SinusoidalStartup
                | Self::ComplementaryPwm
                | Self::StuckRotorProtection
                | Self::AutoTiming
                | Self::StallProtection
                | Self::Telemetry30ms
                | Self::LowVoltageCutoff
                | Self::RcCarReversing
                | Self::UseHallSensors
        )
    }

    /// Get description and valid range for this setting.
    pub const fn description(&self) -> &'static str {
        match self {
            // Throttle/Rate settings
            Self::MaxRateOfChange => "Max rate of change (1-200, display=raw/10)",
            Self::MinDuty => "Min duty cycle (0-50, display=raw/2 %)",
            Self::DisableStickCalibration => "Disable stick calibration (on/off)",
            Self::AbsoluteVoltage => "Absolute voltage raw value (0-255)",
            Self::CurrentLimitP => "Current limit P gain (0-255, display=raw*2)",
            Self::CurrentLimitI => "Current limit I gain (0-255)",
            Self::CurrentLimitD => "Current limit D gain (0-255, display=raw*2)",
            Self::ActiveBrakeStrength => "Active brake strength (1-10)",
            // Motor settings
            Self::DirectionReversed => "Motor direction reversed (on/off)",
            Self::Bidirectional => "3D/bidirectional mode (on/off)",
            Self::SinusoidalStartup => "Sinusoidal startup (on/off)",
            Self::ComplementaryPwm => "Complementary PWM (on/off)",
            Self::VariablePwm => "Variable PWM mode (0=fixed, 1=var, 2=auto)",
            Self::StuckRotorProtection => "Stuck rotor protection (on/off)",
            Self::TimingAdvance => "Motor timing in degrees (0-30, steps of 0.9375°)",
            Self::AutoTiming => "Auto timing adjustment (on/off)",
            Self::PwmFrequency => "PWM frequency in kHz (16-96)",
            Self::StartupPower => "Startup power percentage (50-150)",
            Self::MotorKv => "Motor KV rating (40-10200)",
            Self::MotorPoles => "Motor pole count (2-36)",
            // Brake settings
            Self::BrakeOnStop => "Brake mode (0=off, 1=brake, 2=active)",
            Self::StallProtection => "Stall protection (on/off)",
            Self::DragBrakeStrength => "Drag brake strength (1-10)",
            Self::RunningBrakeStrength => "Running brake strength (1-10)",
            // Audio/telemetry
            Self::BeepVolume => "Beep volume (0-11)",
            Self::Telemetry30ms => "30ms telemetry output (on/off)",
            // Servo input
            Self::ServoLow => "Servo low pulse (0-255, μs=raw*2+750)",
            Self::ServoHigh => "Servo high pulse (0-255, μs=raw*2+1750)",
            Self::ServoNeutral => "Servo neutral (0-255, μs=raw+1374)",
            Self::ServoDeadband => "Servo deadband raw value (0-255)",
            // Limits
            Self::LowVoltageCutoff => "Low voltage cutoff enable (on/off)",
            Self::LvcThreshold => "LVC threshold (0-255, V=(raw+250)/100)",
            Self::TemperatureLimit => "Temperature limit (70-142°C, >140=disabled)",
            Self::CurrentLimit => "Current limit (0-100, A=raw*2, >100=disabled)",
            // Input modes
            Self::RcCarReversing => "RC car reversing mode (on/off)",
            Self::UseHallSensors => "Use Hall sensors (on/off)",
            Self::SignalType => {
                "Signal type (0=auto, 1=dshot, 2=servo, 3=serial, 4=edt+arm, 5=dronecan)"
            }
            // Sine mode
            Self::SineModeRange => "Sine mode range percent (5-25)",
            Self::SineModePower => "Sine mode power (1-10)",
        }
    }
}

/// Parse a value string into bool.
pub fn parse_bool(s: &str) -> Option<bool> {
    let is = |words: &[&str]| words.iter().any(|w| w.eq_ignore_ascii_case(s));
    if is(&["on", "true", "yes", "1", "enabled"]) {
        Some(true)
    } else if is(&["off", "false", "no", "0", "disabled"]) {
        Some(false)
    } else {
        None
    }
}

/// Apply a setting to the config.
///
/// Returns Ok(()) on success, Err with message on invalid value.
pub fn apply_setting(
    config: &mut Am32Config,
    setting: Setting,
    value: &str,
) -> Result<(), &'static str> {
    match setting {
        // Throttle/Rate settings
        Setting::MaxRateOfChange => {
            let v: u8 = value.parse().map_err(|_| "expected 1-200")?;
            if !(1..=200).contains(&v) {
                return Err("max rate of change must be 1-200");
            }
            config.set_max_rate_of_change(v);
        }
        Setting::MinDuty => {
            let v: u8 = value.parse().map_err(|_| "expected 0-50")?;
            if v > 50 {
                return Err("min duty must be 0-50");
            }
            config.set_min_duty(v);
        }
        Setting::DisableStickCalibration => {
            config.set_disable_stick_calibration(parse_bool(value).ok_or("expected on/off")?);
        }
        Setting::AbsoluteVoltage => {
            let v: u8 = value.parse().map_err(|_| "expected 0-255")?;
            config.set_absolute_voltage(v);
        }
        Setting::CurrentLimitP => {
            let v: u8 = value.parse().map_err(|_| "expected 0-255")?;
            config.set_current_limit_p(v);
        }
        Setting::CurrentLimitI => {
            let v: u8 = value.parse().map_err(|_| "expected 0-255")?;
            config.set_current_limit_i(v);
        }
        Setting::CurrentLimitD => {
            let v: u8 = value.parse().map_err(|_| "expected 0-255")?;
            config.set_current_limit_d(v);
        }
        Setting::ActiveBrakeStrength => {
            let v: u8 = value.parse().map_err(|_| "expected 1-10")?;
            if !(1..=10).contains(&v) {
                return Err("active brake strength must be 1-10");
            }
            config.set_active_brake_strength(v);
        }
        // Motor settings
        Setting::DirectionReversed => {
            config.set_direction_reversed(parse_bool(value).ok_or("expected on/off")?);
        }
        Setting::Bidirectional => {
            config.set_bidirectional(parse_bool(value).ok_or("expected on/off")?);
        }
        Setting::SinusoidalStartup => {
            config.set_sinusoidal_startup(parse_bool(value).ok_or("expected on/off")?);
        }
        Setting::ComplementaryPwm => {
            config.set_complementary_pwm(parse_bool(value).ok_or("expected on/off")?);
        }
        Setting::VariablePwm => {
            let v: u8 = value.parse().map_err(|_| "expected 0-2")?;
            if v > 2 {
                return Err("variable PWM must be 0-2");
            }
            config.set_variable_pwm_mode(VariablePwmMode::from_byte(v));
        }
        Setting::StuckRotorProtection => {
            config.set_stuck_rotor_protection(parse_bool(value).ok_or("expected on/off")?);
        }
        Setting::TimingAdvance => {
            let degrees: f32 = value
                .trim_end_matches('°')
                .parse()
                .map_err(|_| "expected degrees (0-30)")?;
            let timing = MotorTiming::from_degrees(degrees).ok_or("timing must be 0-30 degrees")?;
            config.set_timing_advance(timing);
        }
        Setting::AutoTiming => {
            config.set_auto_timing(parse_bool(value).ok_or("expected on/off")?);
        }
        Setting::PwmFrequency => {
            let v: u8 = value.parse().map_err(|_| "expected number")?;
            if !(16..=96).contains(&v) {
                return Err("pwm frequency must be 16-96 kHz");
            }
            config.set_pwm_frequency_khz(v);
        }
        Setting::StartupPower => {
            let v: u8 = value.parse().map_err(|_| "expected 50-150")?;
            if !(50..=150).contains(&v) {
                return Err("startup power must be 50-150");
            }
            config.set_startup_power(v);
        }
        Setting::MotorKv => {
            let v: u16 = value.parse().map_err(|_| "expected number")?;
            if !(40..=10200).contains(&v) {
                return Err("motor KV must be 40-10200");
            }
            config.set_motor_kv(v);
        }
        Setting::MotorPoles => {
            let v: u8 = value.parse().map_err(|_| "expected number")?;
            if !(2..=36).contains(&v) {
                return Err("motor poles must be 2-36");
            }
            config.set_motor_poles(v);
        }
        // Brake settings
        Setting::BrakeOnStop => {
            let v: u8 = value.parse().map_err(|_| "expected 0-2")?;
            if v > 2 {
                return Err("brake mode must be 0-2");
            }
            config.set_brake_mode(BrakeMode::from_byte(v));
        }
        Setting::StallProtection => {
            config.set_stall_protection(parse_bool(value).ok_or("expected on/off")?);
        }
        Setting::DragBrakeStrength => {
            let v: u8 = value.parse().map_err(|_| "expected 1-10")?;
            if !(1..=10).contains(&v) {
                return Err("drag brake strength must be 1-10");
            }
            config.set_drag_brake_strength(v);
        }
        Setting::RunningBrakeStrength => {
            let v: u8 = value.parse().map_err(|_| "expected 1-10")?;
            if !(1..=10).contains(&v) {
                return Err("running brake strength must be 1-10");
            }
            config.set_running_brake_strength(v);
        }
        // Audio/telemetry
        Setting::BeepVolume => {
            let v: u8 = value.parse().map_err(|_| "expected 0-11")?;
            if v > 11 {
                return Err("beep volume must be 0-11");
            }
            config.set_beep_volume(v);
        }
        Setting::Telemetry30ms => {
            config.set_telemetry_30ms(parse_bool(value).ok_or("expected on/off")?);
        }
        // Servo input
        Setting::ServoLow => {
            let v: u8 = value.parse().map_err(|_| "expected 0-255")?;
            config.set_servo_low(v);
        }
        Setting::ServoHigh => {
            let v: u8 = value.parse().map_err(|_| "expected 0-255")?;
            config.set_servo_high(v);
        }
        Setting::ServoNeutral => {
            let v: u8 = value.parse().map_err(|_| "expected 0-255")?;
            config.set_servo_neutral(v);
        }
        Setting::ServoDeadband => {
            let v: u8 = value.parse().map_err(|_| "expected 0-255")?;
            config.set_servo_deadband(v);
        }
        // Limits
        Setting::LowVoltageCutoff => {
            config.set_low_voltage_cutoff(parse_bool(value).ok_or("expected on/off")?);
        }
        Setting::LvcThreshold => {
            let v: u8 = value.parse().map_err(|_| "expected 0-255")?;
            config.set_lvc_threshold(v);
        }
        Setting::TemperatureLimit => {
            let v: u8 = value.parse().map_err(|_| "expected 0-255")?;
            config.set_temperature_limit(v);
        }
        Setting::CurrentLimit => {
            let v: u8 = value.parse().map_err(|_| "expected 0-255")?;
            config.set_current_limit(v);
        }
        // Input modes
        Setting::RcCarReversing => {
            config.set_rc_car_reversing(parse_bool(value).ok_or("expected on/off")?);
        }
        Setting::UseHallSensors => {
            config.set_use_hall_sensors(parse_bool(value).ok_or("expected on/off")?);
        }
        Setting::SignalType => {
            let v: u8 = value.parse().map_err(|_| "expected 0-5")?;
            if v > 5 {
                return Err(
                    "signal type must be 0-5 (0=auto, 1=dshot, 2=servo, 3=serial, 4=edt+arm, 5=dronecan)",
                );
            }
            config.set_signal_type(SignalType::from_byte(v));
        }
        // Sine mode
        Setting::SineModeRange => {
            let v: u8 = value.parse().map_err(|_| "expected 5-25")?;
            if !(5..=25).contains(&v) {
                return Err("sine mode range must be 5-25");
            }
            config.set_sine_mode_range(v);
        }
        Setting::SineModePower => {
            let v: u8 = value.parse().map_err(|_| "expected 1-10")?;
            if !(1..=10).contains(&v) {
                return Err("sine mode power must be 1-10");
            }
            config.set_sine_mode_power(v);
        }
    }
    Ok(())
}

// =============================================================================
// Boot Status & Diagnostics
// =============================================================================

/// ESC boot status for diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootStatus {
    /// ESC is bootable (boot byte = 0x01). Normal operating state.
    Bootable,
    /// ESC is stuck in bootloader (boot byte = 0x00).
    /// This can happen after a failed firmware flash.
    /// Recovery: write config with boot byte restored.
    StuckInBootloader,
    /// Boot byte has unexpected value. May indicate EEPROM corruption.
    Unknown(u8),
}

impl BootStatus {
    /// Check if this status indicates a problem that needs recovery.
    pub const fn needs_recovery(&self) -> bool {
        !matches!(self, BootStatus::Bootable)
    }
}

impl fmt::Display for BootStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BootStatus::Bootable => write!(f, "Bootable"),
            BootStatus::StuckInBootloader => write!(f, "Stuck in bootloader (needs recovery)"),
            BootStatus::Unknown(v) => write!(f, "Unknown boot byte: 0x{:02X}", v),
        }
    }
}

// =============================================================================
// Serde-compatible config data struct
// =============================================================================

/// AM32 configuration data for serialization/deserialization.
///
/// This struct contains all configurable ESC settings in a format suitable
/// for JSON/YAML/TOML export. Use this for saving configs to files, creating
/// presets, or sharing configurations between users.
///
/// # Example
///
/// ```ignore
/// use esc_protocol::am32_config::{Am32Config, Am32ConfigData};
///
/// // Read config from ESC
/// let config = Am32Config::from_bytes(eeprom_bytes);
///
/// // Convert to serializable format
/// let data = Am32ConfigData::from(&config);
/// let json = serde_json::to_string_pretty(&data)?;
///
/// // Later, load from JSON
/// let data: Am32ConfigData = serde_json::from_str(&json)?;
/// let config = Am32Config::from(&data);
/// ```
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Am32ConfigData {
    // Metadata (read-only from ESC, but useful for saved configs)
    /// Firmware version (major, minor)
    pub firmware_version: (u8, u8),
    /// Bootloader version
    pub bootloader_version: u8,
    /// EEPROM format version
    pub eeprom_version: u8,

    // Throttle/Rate settings
    /// Max rate of change (1-200, display = raw/10)
    pub max_rate_of_change: u8,
    /// Min duty cycle (0-50, display = raw/2 %)
    pub min_duty: u8,
    /// Disable stick calibration
    pub disable_stick_calibration: bool,
    /// Absolute voltage raw value
    pub absolute_voltage: u8,
    /// Current limit P gain
    pub current_limit_p: u8,
    /// Current limit I gain
    pub current_limit_i: u8,
    /// Current limit D gain
    pub current_limit_d: u8,
    /// Active brake strength (1-10)
    pub active_brake_strength: u8,

    // Motor settings
    /// Motor direction reversed
    pub direction_reversed: bool,
    /// Bidirectional (3D) mode
    pub bidirectional: bool,
    /// Sinusoidal startup
    pub sinusoidal_startup: bool,
    /// Complementary PWM
    pub complementary_pwm: bool,
    /// Variable PWM mode
    pub variable_pwm_mode: VariablePwmMode,
    /// Stuck rotor protection
    pub stuck_rotor_protection: bool,
    /// Motor timing advance
    pub timing_advance: MotorTiming,
    /// Auto timing adjustment
    pub auto_timing: bool,
    /// PWM frequency in kHz
    pub pwm_frequency_khz: u8,
    /// Startup power (50-150%)
    pub startup_power: u8,
    /// Motor KV rating (actual value, not raw)
    pub motor_kv: u16,
    /// Motor pole count
    pub motor_poles: u8,

    // Brake settings
    /// Brake mode
    pub brake_mode: BrakeMode,
    /// Stall protection
    pub stall_protection: bool,
    /// Drag brake strength (1-10)
    pub drag_brake_strength: u8,
    /// Running brake strength (1-10)
    pub running_brake_strength: u8,

    // Audio/telemetry
    /// Beep volume (0-11)
    pub beep_volume: u8,
    /// 30ms telemetry output
    pub telemetry_30ms: bool,

    // Servo input
    /// Signal input type
    pub signal_type: SignalType,
    /// Servo low raw value
    pub servo_low: u8,
    /// Servo high raw value
    pub servo_high: u8,
    /// Servo neutral raw value
    pub servo_neutral: u8,
    /// Servo deadband
    pub servo_deadband: u8,

    // Limits
    /// Low voltage cutoff enabled
    pub low_voltage_cutoff: bool,
    /// LVC threshold raw value
    pub lvc_threshold: u8,
    /// Temperature limit (>140 = disabled)
    pub temperature_limit: u8,
    /// Current limit raw value (>100 = disabled)
    pub current_limit: u8,

    // Input modes
    /// RC car reversing mode
    pub rc_car_reversing: bool,
    /// Use Hall sensors
    pub use_hall_sensors: bool,

    // Sine mode
    /// Sine mode range (5-25%)
    pub sine_mode_range: u8,
    /// Sine mode power (1-10)
    pub sine_mode_power: u8,
}

impl From<&Am32Config> for Am32ConfigData {
    fn from(config: &Am32Config) -> Self {
        Self {
            // Metadata
            firmware_version: config.firmware_version(),
            bootloader_version: config.bootloader_version(),
            eeprom_version: config.eeprom_version(),

            // Throttle/Rate settings
            max_rate_of_change: config.max_rate_of_change(),
            min_duty: config.min_duty(),
            disable_stick_calibration: config.disable_stick_calibration(),
            absolute_voltage: config.absolute_voltage(),
            current_limit_p: config.current_limit_p(),
            current_limit_i: config.current_limit_i(),
            current_limit_d: config.current_limit_d(),
            active_brake_strength: config.active_brake_strength(),

            // Motor settings
            direction_reversed: config.direction_reversed(),
            bidirectional: config.bidirectional(),
            sinusoidal_startup: config.sinusoidal_startup(),
            complementary_pwm: config.complementary_pwm(),
            variable_pwm_mode: config.variable_pwm_mode(),
            stuck_rotor_protection: config.stuck_rotor_protection(),
            timing_advance: config.timing_advance(),
            auto_timing: config.auto_timing(),
            pwm_frequency_khz: config.pwm_frequency_khz(),
            startup_power: config.startup_power(),
            motor_kv: config.motor_kv(),
            motor_poles: config.motor_poles(),

            // Brake settings
            brake_mode: config.brake_mode(),
            stall_protection: config.stall_protection(),
            drag_brake_strength: config.drag_brake_strength(),
            running_brake_strength: config.running_brake_strength(),

            // Audio/telemetry
            beep_volume: config.beep_volume(),
            telemetry_30ms: config.telemetry_30ms(),

            // Servo input
            signal_type: config.signal_type(),
            servo_low: config.servo_low(),
            servo_high: config.servo_high(),
            servo_neutral: config.servo_neutral(),
            servo_deadband: config.servo_deadband(),

            // Limits
            low_voltage_cutoff: config.low_voltage_cutoff(),
            lvc_threshold: config.lvc_threshold(),
            temperature_limit: config.temperature_limit(),
            current_limit: config.current_limit(),

            // Input modes
            rc_car_reversing: config.rc_car_reversing(),
            use_hall_sensors: config.use_hall_sensors(),

            // Sine mode
            sine_mode_range: config.sine_mode_range(),
            sine_mode_power: config.sine_mode_power(),
        }
    }
}

impl From<&Am32ConfigData> for Am32Config {
    fn from(data: &Am32ConfigData) -> Self {
        let mut config = Am32Config::from_bytes([0u8; 256]);

        // Set boot byte (required for ESC to boot)
        config.raw[0] = 0x01;

        // Metadata (write these so saved config is identifiable)
        config.raw[offsets::FIRMWARE_MAJOR] = data.firmware_version.0;
        config.raw[offsets::FIRMWARE_MINOR] = data.firmware_version.1;
        config.raw[offsets::BOOTLOADER_VERSION] = data.bootloader_version;
        config.raw[offsets::EEPROM_VERSION] = data.eeprom_version;

        // Throttle/Rate settings
        config.set_max_rate_of_change(data.max_rate_of_change);
        config.set_min_duty(data.min_duty);
        config.set_disable_stick_calibration(data.disable_stick_calibration);
        config.set_absolute_voltage(data.absolute_voltage);
        config.set_current_limit_p(data.current_limit_p);
        config.set_current_limit_i(data.current_limit_i);
        config.set_current_limit_d(data.current_limit_d);
        config.set_active_brake_strength(data.active_brake_strength);

        // Motor settings
        config.set_direction_reversed(data.direction_reversed);
        config.set_bidirectional(data.bidirectional);
        config.set_sinusoidal_startup(data.sinusoidal_startup);
        config.set_complementary_pwm(data.complementary_pwm);
        config.set_variable_pwm_mode(data.variable_pwm_mode);
        config.set_stuck_rotor_protection(data.stuck_rotor_protection);
        config.set_timing_advance(data.timing_advance);
        config.set_auto_timing(data.auto_timing);
        config.set_pwm_frequency_khz(data.pwm_frequency_khz);
        config.set_startup_power(data.startup_power);
        config.set_motor_kv(data.motor_kv);
        config.set_motor_poles(data.motor_poles);

        // Brake settings
        config.set_brake_mode(data.brake_mode);
        config.set_stall_protection(data.stall_protection);
        config.set_drag_brake_strength(data.drag_brake_strength);
        config.set_running_brake_strength(data.running_brake_strength);

        // Audio/telemetry
        config.set_beep_volume(data.beep_volume);
        config.set_telemetry_30ms(data.telemetry_30ms);

        // Servo input
        config.set_signal_type(data.signal_type);
        config.set_servo_low(data.servo_low);
        config.set_servo_high(data.servo_high);
        config.set_servo_neutral(data.servo_neutral);
        config.set_servo_deadband(data.servo_deadband);

        // Limits
        config.set_low_voltage_cutoff(data.low_voltage_cutoff);
        config.set_lvc_threshold(data.lvc_threshold);
        config.set_temperature_limit(data.temperature_limit);
        config.set_current_limit(data.current_limit);

        // Input modes
        config.set_rc_car_reversing(data.rc_car_reversing);
        config.set_use_hall_sensors(data.use_hall_sensors);

        // Sine mode
        config.set_sine_mode_range(data.sine_mode_range);
        config.set_sine_mode_power(data.sine_mode_power);

        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AM32 `loadEEpromSettings()`: 0..=3 old (×8), 10..=42 new (−10), else 16.
    #[test]
    fn timing_decodes_like_am32() {
        for b in 0..=3u8 {
            assert_eq!(MotorTiming::from_byte(b).steps(), b << 3, "old byte {b}");
        }
        for b in 10..=42u8 {
            assert_eq!(MotorTiming::from_byte(b).steps(), b - 10, "new byte {b}");
        }
        for b in (4..=9u8).chain(43..=255) {
            assert_eq!(MotorTiming::from_byte(b).steps(), 16, "fallback byte {b}");
        }
        // The eagle's ESCs (AM32 2.20) store 26: 15°, which the old decoder showed as 30°.
        assert_eq!(MotorTiming::from_byte(26), MotorTiming::Deg15);
        assert_eq!(MotorTiming::from_byte(26).degrees(), 15.0);
        assert_eq!(MotorTiming::from_byte(10), MotorTiming::Deg0);
        assert_eq!(MotorTiming::from_byte(42), MotorTiming::Deg30);
    }

    #[test]
    fn timing_degrees_round_to_steps() {
        assert_eq!(
            MotorTiming::from_degrees(18.75).map(MotorTiming::steps),
            Some(20)
        );
        assert_eq!(
            MotorTiming::from_degrees(18.8).map(MotorTiming::steps),
            Some(20)
        );
        assert_eq!(MotorTiming::from_degrees(30.0), Some(MotorTiming::Deg30));
        assert_eq!(MotorTiming::from_degrees(30.1), None);
        assert_eq!(MotorTiming::from_degrees(-1.0), None);
        assert_eq!(MotorTiming::from_steps(33), None);
    }

    /// A byte in the old form stays old where the value has one; otherwise new.
    #[test]
    fn timing_write_keeps_the_stored_encoding() {
        let mut c = Am32Config::from_bytes([0; 256]);
        c.raw[offsets::TIMING_ADVANCE] = 2; // old 15°
        c.set_timing_advance(MotorTiming::Deg22_5);
        assert_eq!(c.raw[offsets::TIMING_ADVANCE], 3);
        c.set_timing_advance(MotorTiming::from_degrees(18.75).unwrap());
        assert_eq!(c.raw[offsets::TIMING_ADVANCE], 30);
        // Now new form: stays new even for a value the old form has.
        c.set_timing_advance(MotorTiming::Deg15);
        assert_eq!(c.raw[offsets::TIMING_ADVANCE], 26);
        // 30° has no old form.
        c.raw[offsets::TIMING_ADVANCE] = 0;
        c.set_timing_advance(MotorTiming::Deg30);
        assert_eq!(c.raw[offsets::TIMING_ADVANCE], 42);
        for b in 0..=255u8 {
            c.raw[offsets::TIMING_ADVANCE] = b;
            let t = c.timing_advance();
            c.set_timing_advance(t);
            assert_eq!(c.timing_advance(), t, "byte {b} round trip");
        }
    }

    #[test]
    fn timing_setting_takes_degrees() {
        let mut c = Am32Config::from_bytes([0; 256]);
        c.raw[offsets::TIMING_ADVANCE] = 26;
        apply_setting(&mut c, Setting::TimingAdvance, "18.75").unwrap();
        assert_eq!(c.timing_advance().steps(), 20);
        apply_setting(&mut c, Setting::TimingAdvance, "15°").unwrap();
        assert_eq!(c.timing_advance(), MotorTiming::Deg15);
        assert!(apply_setting(&mut c, Setting::TimingAdvance, "31").is_err());
    }

    #[cfg(feature = "serde")]
    #[test]
    fn timing_serde() {
        #[derive(serde::Deserialize, serde::Serialize)]
        struct W {
            t: MotorTiming,
        }
        let w: W = toml::from_str("t = 15").unwrap();
        assert_eq!(w.t, MotorTiming::Deg15);
        let w: W = toml::from_str("t = 18.75").unwrap();
        assert_eq!(w.t.steps(), 20);
        let w: W = toml::from_str("t = \"Deg22_5\"").unwrap();
        assert_eq!(w.t, MotorTiming::Deg22_5);
        assert!(toml::from_str::<W>("t = \"Deg30\"").is_err());
        assert!(toml::from_str::<W>("t = 31").is_err());
        let back: W = toml::from_str(
            &toml::to_string(&W {
                t: MotorTiming::Deg15,
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(back.t, MotorTiming::Deg15);
    }

    #[test]
    fn parse_config() {
        // Sample config from user's ESC
        let mut raw = [0xFF; 256];
        raw[0x00] = 0x01; // boot byte
        raw[0x01] = 0x01; // eeprom version
        raw[0x02] = 0x01; // bootloader version
        raw[0x03] = 0x02; // fw major
        raw[0x04] = 0x0C; // fw minor (12)
        raw[0x17] = 0x02; // timing (15°)
        raw[0x18] = 0x30; // pwm freq (48 kHz)
        raw[0x19] = 0x64; // startup power (100%)
        raw[0x1A] = 0x16; // motor kv raw (22 * 40 = 880)
        raw[0x1B] = 0x0E; // motor poles (14)
        raw[0x1E] = 0x05; // beep volume

        let config = Am32Config::from_bytes(raw);

        assert_eq!(config.firmware_version(), (2, 12));
        assert_eq!(config.timing_advance(), MotorTiming::Deg15);
        assert_eq!(config.pwm_frequency_khz(), 48);
        assert_eq!(config.startup_power(), 100);
        assert_eq!(config.motor_kv(), 880);
        assert_eq!(config.motor_poles(), 14);
        assert_eq!(config.beep_volume(), 5);
    }

    #[test]
    fn setting_from_str() {
        assert_eq!(
            Setting::parse("direction"),
            Some(Setting::DirectionReversed)
        );
        assert_eq!(Setting::parse("motor-kv"), Some(Setting::MotorKv));
        assert_eq!(Setting::parse("pwm_freq"), Some(Setting::PwmFrequency));
        assert_eq!(Setting::parse("unknown"), None);
    }

    #[test]
    fn apply_bool_setting() {
        let mut config = Am32Config::from_bytes([0; 256]);
        apply_setting(&mut config, Setting::DirectionReversed, "on").unwrap();
        assert!(config.direction_reversed());
        apply_setting(&mut config, Setting::DirectionReversed, "off").unwrap();
        assert!(!config.direction_reversed());
    }

    #[test]
    fn apply_numeric_setting() {
        let mut config = Am32Config::from_bytes([0; 256]);
        apply_setting(&mut config, Setting::MotorKv, "2200").unwrap();
        assert_eq!(config.motor_kv(), 2200);
        apply_setting(&mut config, Setting::BeepVolume, "5").unwrap();
        assert_eq!(config.beep_volume(), 5);
    }

    #[test]
    fn config_data_roundtrip() {
        // Create a config with specific values
        let mut config = Am32Config::from_bytes([0; 256]);
        config.raw[0] = 0x01; // boot byte

        // Set all configurable fields to non-default values
        config.set_max_rate_of_change(150);
        config.set_min_duty(25);
        config.set_disable_stick_calibration(true);
        config.set_absolute_voltage(42);
        config.set_current_limit_p(100);
        config.set_current_limit_i(50);
        config.set_current_limit_d(75);
        config.set_active_brake_strength(8);

        config.set_direction_reversed(true);
        config.set_bidirectional(true);
        config.set_sinusoidal_startup(true);
        config.set_complementary_pwm(true);
        config.set_variable_pwm_mode(VariablePwmMode::Auto);
        config.set_stuck_rotor_protection(true);
        config.set_timing_advance(MotorTiming::Deg22_5);
        config.set_auto_timing(true);
        config.set_pwm_frequency_khz(48);
        config.set_startup_power(125);
        config.set_motor_kv(2200);
        config.set_motor_poles(14);

        config.set_brake_mode(BrakeMode::ActiveBrake);
        config.set_stall_protection(true);
        config.set_drag_brake_strength(7);
        config.set_running_brake_strength(6);

        config.set_beep_volume(8);
        config.set_telemetry_30ms(true);

        config.set_signal_type(SignalType::DShot);
        config.set_servo_low(100);
        config.set_servo_high(200);
        config.set_servo_neutral(128);
        config.set_servo_deadband(10);

        config.set_low_voltage_cutoff(true);
        config.set_lvc_threshold(100);
        config.set_temperature_limit(120);
        config.set_current_limit(50);

        config.set_rc_car_reversing(true);
        config.set_use_hall_sensors(true);

        config.set_sine_mode_range(15);
        config.set_sine_mode_power(7);

        // Convert to data and back
        let data = Am32ConfigData::from(&config);
        let config2 = Am32Config::from(&data);

        // Verify all fields match
        assert_eq!(config2.max_rate_of_change(), 150);
        assert_eq!(config2.min_duty(), 25);
        assert!(config2.disable_stick_calibration());
        assert_eq!(config2.absolute_voltage(), 42);
        assert_eq!(config2.current_limit_p(), 100);
        assert_eq!(config2.current_limit_i(), 50);
        assert_eq!(config2.current_limit_d(), 75);
        assert_eq!(config2.active_brake_strength(), 8);

        assert!(config2.direction_reversed());
        assert!(config2.bidirectional());
        assert!(config2.sinusoidal_startup());
        assert!(config2.complementary_pwm());
        assert_eq!(config2.variable_pwm_mode(), VariablePwmMode::Auto);
        assert!(config2.stuck_rotor_protection());
        assert_eq!(config2.timing_advance(), MotorTiming::Deg22_5);
        assert!(config2.auto_timing());
        assert_eq!(config2.pwm_frequency_khz(), 48);
        assert_eq!(config2.startup_power(), 125);
        assert_eq!(config2.motor_kv(), 2200);
        assert_eq!(config2.motor_poles(), 14);

        assert_eq!(config2.brake_mode(), BrakeMode::ActiveBrake);
        assert!(config2.stall_protection());
        assert_eq!(config2.drag_brake_strength(), 7);
        assert_eq!(config2.running_brake_strength(), 6);

        assert_eq!(config2.beep_volume(), 8);
        assert!(config2.telemetry_30ms());

        assert_eq!(config2.signal_type(), SignalType::DShot);
        assert_eq!(config2.servo_low(), 100);
        assert_eq!(config2.servo_high(), 200);
        assert_eq!(config2.servo_neutral(), 128);
        assert_eq!(config2.servo_deadband(), 10);

        assert!(config2.low_voltage_cutoff());
        assert_eq!(config2.lvc_threshold(), 100);
        assert_eq!(config2.temperature_limit(), 120);
        assert_eq!(config2.current_limit(), 50);

        assert!(config2.rc_car_reversing());
        assert!(config2.use_hall_sensors());

        assert_eq!(config2.sine_mode_range(), 15);
        assert_eq!(config2.sine_mode_power(), 7);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn config_data_toml_roundtrip() {
        // Create a config with specific values
        let mut config = Am32Config::from_bytes([0; 256]);
        config.raw[0] = 0x01;
        config.raw[offsets::FIRMWARE_MAJOR] = 2;
        config.raw[offsets::FIRMWARE_MINOR] = 15;
        config.set_motor_kv(2200);
        config.set_motor_poles(14);
        config.set_timing_advance(MotorTiming::Deg15);
        config.set_direction_reversed(true);
        config.set_bidirectional(true);
        config.set_brake_mode(BrakeMode::ActiveBrake);
        config.set_signal_type(SignalType::DShot);
        config.set_beep_volume(5);

        // Serialize to TOML
        let data = Am32ConfigData::from(&config);
        let toml_str = toml::to_string_pretty(&data).expect("Failed to serialize to TOML");

        // Verify TOML contains expected values
        assert!(toml_str.contains("motor_kv = 2200"));
        assert!(toml_str.contains("motor_poles = 14"));
        assert!(toml_str.contains("direction_reversed = true"));
        assert!(toml_str.contains("bidirectional = true"));

        // Deserialize back
        let data2: Am32ConfigData =
            toml::from_str(&toml_str).expect("Failed to deserialize from TOML");
        let config2 = Am32Config::from(&data2);

        // Verify all fields match
        assert_eq!(config2.firmware_version(), (2, 15));
        assert_eq!(config2.motor_kv(), 2200);
        assert_eq!(config2.motor_poles(), 14);
        assert_eq!(config2.timing_advance(), MotorTiming::Deg15);
        assert!(config2.direction_reversed());
        assert!(config2.bidirectional());
        assert_eq!(config2.brake_mode(), BrakeMode::ActiveBrake);
        assert_eq!(config2.signal_type(), SignalType::DShot);
        assert_eq!(config2.beep_volume(), 5);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn config_data_default_values() {
        // Test that default values are sensible
        let data = Am32ConfigData::default();
        let config = Am32Config::from(&data);

        // Verify defaults don't cause issues
        assert_eq!(config.boot_byte(), 0x01); // Boot byte should be set
        assert_eq!(config.variable_pwm_mode(), VariablePwmMode::Fixed);
        assert_eq!(config.brake_mode(), BrakeMode::Off);
        assert_eq!(config.signal_type(), SignalType::Auto);
        assert_eq!(config.timing_advance(), MotorTiming::Deg0);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn config_data_partial_toml() {
        // Test deserializing partial TOML (missing fields should use defaults)
        let partial_toml = r#"
motor_kv = 1800
motor_poles = 12
direction_reversed = true
"#;

        let data: Am32ConfigData =
            toml::from_str(partial_toml).expect("Failed to deserialize partial TOML");
        let config = Am32Config::from(&data);

        // Specified values should be set
        assert_eq!(config.motor_kv(), 1800);
        assert_eq!(config.motor_poles(), 12);
        assert!(config.direction_reversed());

        // Unspecified values should be defaults
        assert!(!config.bidirectional());
        assert_eq!(config.brake_mode(), BrakeMode::Off);
    }

    // =========================================================================
    // Safety Bit Handling Tests
    // =========================================================================

    #[test]
    fn boot_byte_constants() {
        assert_eq!(Am32Config::BOOT_BYTE_ENABLED, 0x01);
        assert_eq!(Am32Config::BOOT_BYTE_DISABLED, 0x00);
    }

    #[test]
    fn is_bootable() {
        let mut config = Am32Config::from_bytes([0; 256]);

        // Initially not bootable (boot byte = 0)
        assert!(!config.is_bootable());

        // Set boot byte to enabled
        config.set_boot_byte(Am32Config::BOOT_BYTE_ENABLED);
        assert!(config.is_bootable());

        // Set boot byte to disabled
        config.set_boot_byte(Am32Config::BOOT_BYTE_DISABLED);
        assert!(!config.is_bootable());
    }

    #[test]
    fn clear_and_restore_boot_byte() {
        let mut config = Am32Config::from_bytes([0; 256]);
        config.set_boot_byte(0x01);
        assert!(config.is_bootable());

        // Clear should return previous value and set to 0
        let previous = config.clear_boot_byte();
        assert_eq!(previous, 0x01);
        assert_eq!(config.boot_byte(), 0x00);
        assert!(!config.is_bootable());

        // Restore should set to 1
        config.restore_boot_byte();
        assert_eq!(config.boot_byte(), 0x01);
        assert!(config.is_bootable());
    }

    #[test]
    fn eeprom_bytes_access() {
        let mut raw = [0xAAu8; 256];
        raw[0] = 0x01; // boot byte
        raw[47] = 0xFF; // last byte of EEPROM region
        let config = Am32Config::from_bytes(raw);

        let eeprom = config.eeprom_bytes();
        assert_eq!(eeprom.len(), 48);
        assert_eq!(eeprom[0], 0x01);
        assert_eq!(eeprom[47], 0xFF);
    }

    #[test]
    fn defaults_are_bootable_and_current() {
        // F1: byte 0 = 0 left the ESC stuck in the bootloader.
        let config = Am32Config::with_defaults();
        assert_eq!(config.boot_status(), BootStatus::Bootable);
        assert_eq!(config.eeprom_version(), Am32Config::CURRENT_EEPROM_VERSION);
        // Melody and CAN erased, not zeroed.
        assert!(config.as_bytes()[0x30..0xC0].iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn apply_defaults_keeps_melody_can_and_versions() {
        let mut raw = [0u8; 256];
        raw[0] = 0x00; // boot disabled
        raw[1] = 5; // newer eeprom version: keep
        raw[3] = 2;
        raw[4] = 21;
        raw[0x30..0xB0].fill(0x42);
        raw[0xB0..0xB8].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let mut config = Am32Config::from_bytes(raw);
        config.apply_defaults();

        let b = config.as_bytes();
        assert_eq!(b[0], 0x01);
        assert_eq!(b[1], 5);
        assert_eq!((b[3], b[4]), (2, 21));
        assert!(b[0x30..0xB0].iter().all(|&x| x == 0x42));
        assert_eq!(&b[0xB0..0xB8], &[1, 2, 3, 4, 5, 6, 7, 8]);

        let mut old = Am32Config::from_bytes([0u8; 256]);
        old.as_bytes_mut()[1] = 2;
        old.apply_defaults();
        assert_eq!(old.eeprom_version(), Am32Config::CURRENT_EEPROM_VERSION);
    }

    #[test]
    fn boot_status_bootable() {
        let mut config = Am32Config::from_bytes([0; 256]);
        config.set_boot_byte(0x01);

        assert_eq!(config.boot_status(), BootStatus::Bootable);
        assert!(!config.boot_status().needs_recovery());
    }

    #[test]
    fn boot_status_stuck_in_bootloader() {
        let config = Am32Config::from_bytes([0; 256]); // boot byte = 0

        assert_eq!(config.boot_status(), BootStatus::StuckInBootloader);
        assert!(config.boot_status().needs_recovery());
    }

    #[test]
    fn boot_status_unknown() {
        let mut config = Am32Config::from_bytes([0; 256]);
        config.set_boot_byte(0xFF); // unexpected value

        assert_eq!(config.boot_status(), BootStatus::Unknown(0xFF));
        assert!(config.boot_status().needs_recovery());
    }

    #[test]
    fn boot_status_display() {
        use std::string::ToString;

        let mut config = Am32Config::from_bytes([0; 256]);

        config.set_boot_byte(0x01);
        assert_eq!(config.boot_status().to_string(), "Bootable");

        config.set_boot_byte(0x00);
        assert!(config.boot_status().to_string().contains("Stuck"));

        config.set_boot_byte(0xAB);
        assert!(config.boot_status().to_string().contains("0xAB"));
    }

    #[test]
    fn recovery_workflow() {
        // Simulate diagnosing and recovering a stuck ESC
        let mut config = Am32Config::from_bytes([0; 256]);

        // ESC is stuck (boot byte = 0)
        assert_eq!(config.boot_status(), BootStatus::StuckInBootloader);
        assert!(config.boot_status().needs_recovery());

        // Recovery: restore boot byte
        config.restore_boot_byte();

        // Now bootable
        assert_eq!(config.boot_status(), BootStatus::Bootable);
        assert!(!config.boot_status().needs_recovery());
    }
}
