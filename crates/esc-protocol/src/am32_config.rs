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

/// Motor timing advance options (degrees)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotorTiming {
    Deg0 = 0,
    Deg7_5 = 1,
    Deg15 = 2,
    Deg22_5 = 3,
    Deg30 = 4,
}

impl MotorTiming {
    pub fn from_byte(b: u8) -> Self {
        match b {
            0 => Self::Deg0,
            1 => Self::Deg7_5,
            2 => Self::Deg15,
            3 => Self::Deg22_5,
            _ => Self::Deg30,
        }
    }

    pub fn degrees(self) -> f32 {
        match self {
            Self::Deg0 => 0.0,
            Self::Deg7_5 => 7.5,
            Self::Deg15 => 15.0,
            Self::Deg22_5 => 22.5,
            Self::Deg30 => 30.0,
        }
    }
}

impl fmt::Display for MotorTiming {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}°", self.degrees())
    }
}

// EEPROM byte offsets
mod offsets {
    pub const BOOT_BYTE: usize = 0x00;
    pub const EEPROM_VERSION: usize = 0x01;
    pub const BOOTLOADER_VERSION: usize = 0x02;
    pub const FIRMWARE_MAJOR: usize = 0x03;
    pub const FIRMWARE_MINOR: usize = 0x04;
    pub const ESC_NAME: usize = 0x05; // 12 bytes
    pub const ESC_NAME_LEN: usize = 12;
    pub const DIRECTION_REVERSED: usize = 0x11;
    pub const BIDIRECTIONAL: usize = 0x12;
    pub const SINUSOIDAL_STARTUP: usize = 0x13;
    pub const COMPLEMENTARY_PWM: usize = 0x14;
    pub const VARIABLE_PWM: usize = 0x15;
    pub const STUCK_ROTOR_PROTECTION: usize = 0x16;
    pub const TIMING_ADVANCE: usize = 0x17;
    pub const PWM_FREQUENCY: usize = 0x18;
    pub const STARTUP_POWER: usize = 0x19;
    pub const MOTOR_KV: usize = 0x1A;
    pub const MOTOR_POLES: usize = 0x1B;
    pub const BRAKE_ON_STOP: usize = 0x1C;
    pub const STALL_PROTECTION: usize = 0x1D;
    pub const BEEP_VOLUME: usize = 0x1E;
    pub const TELEMETRY_30MS: usize = 0x1F;
    pub const SERVO_LOW: usize = 0x20;
    pub const SERVO_HIGH: usize = 0x21;
    pub const SERVO_NEUTRAL: usize = 0x22;
    pub const LOW_VOLTAGE_CUTOFF: usize = 0x23;
    pub const LVC_THRESHOLD: usize = 0x24;
    pub const RC_CAR_REVERSING: usize = 0x25;
    pub const USE_HALL_SENSORS: usize = 0x26;
    pub const SINE_MODE_RANGE: usize = 0x28;
    pub const DRAG_BRAKE_STRENGTH: usize = 0x29;
}

impl Am32Config {
    /// Create config from raw 256-byte array.
    pub const fn from_bytes(raw: [u8; 256]) -> Self {
        Self { raw }
    }

    /// Get raw bytes.
    pub const fn as_bytes(&self) -> &[u8; 256] {
        &self.raw
    }

    /// Get mutable raw bytes.
    pub fn as_bytes_mut(&mut self) -> &mut [u8; 256] {
        &mut self.raw
    }

    // =========================================================================
    // Read-only getters
    // =========================================================================

    /// ESC boot byte (must be 1 for ESC to boot).
    pub fn boot_byte(&self) -> u8 {
        self.raw[offsets::BOOT_BYTE]
    }

    /// EEPROM format version.
    pub fn eeprom_version(&self) -> u8 {
        self.raw[offsets::EEPROM_VERSION]
    }

    /// Bootloader version.
    pub fn bootloader_version(&self) -> u8 {
        self.raw[offsets::BOOTLOADER_VERSION]
    }

    /// Firmware version as (major, minor).
    pub fn firmware_version(&self) -> (u8, u8) {
        (
            self.raw[offsets::FIRMWARE_MAJOR],
            self.raw[offsets::FIRMWARE_MINOR],
        )
    }

    /// ESC name (up to 12 chars, trimmed).
    pub fn name(&self) -> &str {
        let bytes = &self.raw[offsets::ESC_NAME..offsets::ESC_NAME + offsets::ESC_NAME_LEN];
        // Find end of string (null or spaces)
        let end = bytes
            .iter()
            .rposition(|&b| b != 0 && b != b' ')
            .map(|i| i + 1)
            .unwrap_or(0);
        core::str::from_utf8(&bytes[..end]).unwrap_or("<invalid>")
    }

    // =========================================================================
    // Configurable settings with getters and setters
    // =========================================================================

    /// Motor direction reversed.
    pub fn direction_reversed(&self) -> bool {
        self.raw[offsets::DIRECTION_REVERSED] != 0
    }

    pub fn set_direction_reversed(&mut self, value: bool) {
        self.raw[offsets::DIRECTION_REVERSED] = value as u8;
    }

    /// Bidirectional mode (3D mode).
    pub fn bidirectional(&self) -> bool {
        self.raw[offsets::BIDIRECTIONAL] != 0
    }

    pub fn set_bidirectional(&mut self, value: bool) {
        self.raw[offsets::BIDIRECTIONAL] = value as u8;
    }

    /// Sinusoidal startup mode.
    pub fn sinusoidal_startup(&self) -> bool {
        self.raw[offsets::SINUSOIDAL_STARTUP] != 0
    }

    pub fn set_sinusoidal_startup(&mut self, value: bool) {
        self.raw[offsets::SINUSOIDAL_STARTUP] = value as u8;
    }

    /// Complementary PWM.
    pub fn complementary_pwm(&self) -> bool {
        self.raw[offsets::COMPLEMENTARY_PWM] != 0
    }

    pub fn set_complementary_pwm(&mut self, value: bool) {
        self.raw[offsets::COMPLEMENTARY_PWM] = value as u8;
    }

    /// Variable PWM frequency.
    pub fn variable_pwm(&self) -> bool {
        self.raw[offsets::VARIABLE_PWM] != 0
    }

    pub fn set_variable_pwm(&mut self, value: bool) {
        self.raw[offsets::VARIABLE_PWM] = value as u8;
    }

    /// Stuck rotor protection.
    pub fn stuck_rotor_protection(&self) -> bool {
        self.raw[offsets::STUCK_ROTOR_PROTECTION] != 0
    }

    pub fn set_stuck_rotor_protection(&mut self, value: bool) {
        self.raw[offsets::STUCK_ROTOR_PROTECTION] = value as u8;
    }

    /// Motor timing advance.
    pub fn timing_advance(&self) -> MotorTiming {
        MotorTiming::from_byte(self.raw[offsets::TIMING_ADVANCE])
    }

    pub fn set_timing_advance(&mut self, timing: MotorTiming) {
        self.raw[offsets::TIMING_ADVANCE] = timing as u8;
    }

    /// PWM frequency in kHz.
    pub fn pwm_frequency_khz(&self) -> u8 {
        self.raw[offsets::PWM_FREQUENCY]
    }

    pub fn set_pwm_frequency_khz(&mut self, khz: u8) {
        self.raw[offsets::PWM_FREQUENCY] = khz;
    }

    /// Startup power (50-150%).
    pub fn startup_power(&self) -> u8 {
        self.raw[offsets::STARTUP_POWER]
    }

    pub fn set_startup_power(&mut self, power: u8) {
        self.raw[offsets::STARTUP_POWER] = power.clamp(50, 150);
    }

    /// Motor KV (raw value * 40 = actual KV).
    pub fn motor_kv_raw(&self) -> u8 {
        self.raw[offsets::MOTOR_KV]
    }

    /// Motor KV in actual value.
    pub fn motor_kv(&self) -> u16 {
        self.raw[offsets::MOTOR_KV] as u16 * 40
    }

    pub fn set_motor_kv(&mut self, kv: u16) {
        self.raw[offsets::MOTOR_KV] = (kv / 40).min(255) as u8;
    }

    /// Motor poles.
    pub fn motor_poles(&self) -> u8 {
        self.raw[offsets::MOTOR_POLES]
    }

    pub fn set_motor_poles(&mut self, poles: u8) {
        self.raw[offsets::MOTOR_POLES] = poles;
    }

    /// Brake on stop.
    pub fn brake_on_stop(&self) -> bool {
        self.raw[offsets::BRAKE_ON_STOP] != 0
    }

    pub fn set_brake_on_stop(&mut self, value: bool) {
        self.raw[offsets::BRAKE_ON_STOP] = value as u8;
    }

    /// Stall protection.
    pub fn stall_protection(&self) -> bool {
        self.raw[offsets::STALL_PROTECTION] != 0
    }

    pub fn set_stall_protection(&mut self, value: bool) {
        self.raw[offsets::STALL_PROTECTION] = value as u8;
    }

    /// Beep volume (0-11).
    pub fn beep_volume(&self) -> u8 {
        self.raw[offsets::BEEP_VOLUME]
    }

    pub fn set_beep_volume(&mut self, volume: u8) {
        self.raw[offsets::BEEP_VOLUME] = volume.min(11);
    }

    /// 30ms telemetry output.
    pub fn telemetry_30ms(&self) -> bool {
        self.raw[offsets::TELEMETRY_30MS] != 0
    }

    pub fn set_telemetry_30ms(&mut self, value: bool) {
        self.raw[offsets::TELEMETRY_30MS] = value as u8;
    }

    /// Servo low value. Actual microseconds = (value * 2) + 750.
    pub fn servo_low(&self) -> u8 {
        self.raw[offsets::SERVO_LOW]
    }

    pub fn servo_low_us(&self) -> u16 {
        self.raw[offsets::SERVO_LOW] as u16 * 2 + 750
    }

    pub fn set_servo_low(&mut self, value: u8) {
        self.raw[offsets::SERVO_LOW] = value;
    }

    /// Servo high value. Actual microseconds = (value * 2) + 1750.
    pub fn servo_high(&self) -> u8 {
        self.raw[offsets::SERVO_HIGH]
    }

    pub fn servo_high_us(&self) -> u16 {
        self.raw[offsets::SERVO_HIGH] as u16 * 2 + 1750
    }

    pub fn set_servo_high(&mut self, value: u8) {
        self.raw[offsets::SERVO_HIGH] = value;
    }

    /// Servo neutral. Actual microseconds = 1374 + value.
    pub fn servo_neutral(&self) -> u8 {
        self.raw[offsets::SERVO_NEUTRAL]
    }

    pub fn servo_neutral_us(&self) -> u16 {
        1374 + self.raw[offsets::SERVO_NEUTRAL] as u16
    }

    pub fn set_servo_neutral(&mut self, value: u8) {
        self.raw[offsets::SERVO_NEUTRAL] = value;
    }

    /// Low voltage cutoff enabled.
    pub fn low_voltage_cutoff(&self) -> bool {
        self.raw[offsets::LOW_VOLTAGE_CUTOFF] != 0
    }

    pub fn set_low_voltage_cutoff(&mut self, value: bool) {
        self.raw[offsets::LOW_VOLTAGE_CUTOFF] = value as u8;
    }

    /// Low voltage cutoff threshold. Actual voltage = 2.5 + (value * 0.1).
    pub fn lvc_threshold(&self) -> u8 {
        self.raw[offsets::LVC_THRESHOLD]
    }

    pub fn lvc_threshold_volts(&self) -> f32 {
        2.5 + self.raw[offsets::LVC_THRESHOLD] as f32 * 0.1
    }

    pub fn set_lvc_threshold(&mut self, value: u8) {
        self.raw[offsets::LVC_THRESHOLD] = value;
    }

    /// RC car style reversing.
    pub fn rc_car_reversing(&self) -> bool {
        self.raw[offsets::RC_CAR_REVERSING] != 0
    }

    pub fn set_rc_car_reversing(&mut self, value: bool) {
        self.raw[offsets::RC_CAR_REVERSING] = value as u8;
    }

    /// Use Hall sensors (if equipped).
    pub fn use_hall_sensors(&self) -> bool {
        self.raw[offsets::USE_HALL_SENSORS] != 0
    }

    pub fn set_use_hall_sensors(&mut self, value: bool) {
        self.raw[offsets::USE_HALL_SENSORS] = value as u8;
    }

    /// Sine mode range (5-25% throttle).
    pub fn sine_mode_range(&self) -> u8 {
        self.raw[offsets::SINE_MODE_RANGE]
    }

    pub fn set_sine_mode_range(&mut self, value: u8) {
        self.raw[offsets::SINE_MODE_RANGE] = value.clamp(5, 25);
    }

    /// Drag brake strength (1-10).
    pub fn drag_brake_strength(&self) -> u8 {
        self.raw[offsets::DRAG_BRAKE_STRENGTH]
    }

    pub fn set_drag_brake_strength(&mut self, value: u8) {
        self.raw[offsets::DRAG_BRAKE_STRENGTH] = value.clamp(1, 10);
    }
}

impl fmt::Display for Am32Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (fw_major, fw_minor) = self.firmware_version();
        writeln!(f, "AM32 ESC Configuration")?;
        writeln!(f, "======================")?;
        writeln!(f)?;
        writeln!(f, "ESC Name:            {}", self.name())?;
        writeln!(f, "Firmware:            v{}.{}", fw_major, fw_minor)?;
        writeln!(f, "Bootloader:          v{}", self.bootloader_version())?;
        writeln!(f, "EEPROM Version:      {}", self.eeprom_version())?;
        writeln!(f)?;
        writeln!(f, "Motor Settings")?;
        writeln!(f, "--------------")?;
        writeln!(f, "Direction Reversed:  {}", if self.direction_reversed() { "Yes" } else { "No" })?;
        writeln!(f, "Bidirectional (3D):  {}", if self.bidirectional() { "Yes" } else { "No" })?;
        writeln!(f, "Motor KV:            {} (raw: {})", self.motor_kv(), self.motor_kv_raw())?;
        writeln!(f, "Motor Poles:         {}", self.motor_poles())?;
        writeln!(f, "Timing Advance:      {}", self.timing_advance())?;
        writeln!(f)?;
        writeln!(f, "PWM Settings")?;
        writeln!(f, "------------")?;
        writeln!(f, "PWM Frequency:       {} kHz", self.pwm_frequency_khz())?;
        writeln!(f, "Complementary PWM:   {}", if self.complementary_pwm() { "Yes" } else { "No" })?;
        writeln!(f, "Variable PWM:        {}", if self.variable_pwm() { "Yes" } else { "No" })?;
        writeln!(f)?;
        writeln!(f, "Startup & Protection")?;
        writeln!(f, "--------------------")?;
        writeln!(f, "Startup Power:       {}%", self.startup_power())?;
        writeln!(f, "Sinusoidal Startup:  {}", if self.sinusoidal_startup() { "Yes" } else { "No" })?;
        writeln!(f, "Stuck Rotor Prot.:   {}", if self.stuck_rotor_protection() { "Yes" } else { "No" })?;
        writeln!(f, "Stall Protection:    {}", if self.stall_protection() { "Yes" } else { "No" })?;
        writeln!(f)?;
        writeln!(f, "Braking")?;
        writeln!(f, "-------")?;
        writeln!(f, "Brake on Stop:       {}", if self.brake_on_stop() { "Yes" } else { "No" })?;
        writeln!(f, "Drag Brake Strength: {}/10", self.drag_brake_strength())?;
        writeln!(f)?;
        writeln!(f, "Low Voltage Cutoff")?;
        writeln!(f, "------------------")?;
        writeln!(f, "LVC Enabled:         {}", if self.low_voltage_cutoff() { "Yes" } else { "No" })?;
        writeln!(f, "LVC Threshold:       {:.1}V (raw: {})", self.lvc_threshold_volts(), self.lvc_threshold())?;
        writeln!(f)?;
        writeln!(f, "Servo / PWM Input")?;
        writeln!(f, "-----------------")?;
        writeln!(f, "Servo Low:           {} us (raw: {})", self.servo_low_us(), self.servo_low())?;
        writeln!(f, "Servo High:          {} us (raw: {})", self.servo_high_us(), self.servo_high())?;
        writeln!(f, "Servo Neutral:       {} us (raw: {})", self.servo_neutral_us(), self.servo_neutral())?;
        writeln!(f)?;
        writeln!(f, "Other")?;
        writeln!(f, "-----")?;
        writeln!(f, "Beep Volume:         {}/11", self.beep_volume())?;
        writeln!(f, "30ms Telemetry:      {}", if self.telemetry_30ms() { "Yes" } else { "No" })?;
        writeln!(f, "RC Car Reversing:    {}", if self.rc_car_reversing() { "Yes" } else { "No" })?;
        writeln!(f, "Use Hall Sensors:    {}", if self.use_hall_sensors() { "Yes" } else { "No" })?;
        writeln!(f, "Sine Mode Range:     {}%", self.sine_mode_range())?;
        Ok(())
    }
}

/// Setting name for CLI commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    DirectionReversed,
    Bidirectional,
    SinusoidalStartup,
    ComplementaryPwm,
    VariablePwm,
    StuckRotorProtection,
    TimingAdvance,
    PwmFrequency,
    StartupPower,
    MotorKv,
    MotorPoles,
    BrakeOnStop,
    StallProtection,
    BeepVolume,
    Telemetry30ms,
    ServoLow,
    ServoHigh,
    ServoNeutral,
    LowVoltageCutoff,
    LvcThreshold,
    RcCarReversing,
    UseHallSensors,
    SineModeRange,
    DragBrakeStrength,
}

impl Setting {
    /// Parse setting name from string.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.to_lowercase().replace(['-', '_'], "");
        match s.as_str() {
            "directionreversed" | "direction" | "reversed" | "dir" => Some(Self::DirectionReversed),
            "bidirectional" | "bidir" | "3d" | "3dmode" => Some(Self::Bidirectional),
            "sinusoidalstartup" | "sinusoidal" | "sinestart" => Some(Self::SinusoidalStartup),
            "complementarypwm" | "comppwm" => Some(Self::ComplementaryPwm),
            "variablepwm" | "varpwm" => Some(Self::VariablePwm),
            "stuckrotorprotection" | "stuckrotor" => Some(Self::StuckRotorProtection),
            "timingadvance" | "timing" => Some(Self::TimingAdvance),
            "pwmfrequency" | "pwmfreq" | "pwm" => Some(Self::PwmFrequency),
            "startuppower" | "startup" => Some(Self::StartupPower),
            "motorkv" | "kv" => Some(Self::MotorKv),
            "motorpoles" | "poles" => Some(Self::MotorPoles),
            "brakeonstop" | "brake" => Some(Self::BrakeOnStop),
            "stallprotection" | "stall" => Some(Self::StallProtection),
            "beepvolume" | "beep" | "volume" => Some(Self::BeepVolume),
            "telemetry30ms" | "telemetry" | "telem" => Some(Self::Telemetry30ms),
            "servolow" => Some(Self::ServoLow),
            "servohigh" => Some(Self::ServoHigh),
            "servoneutral" | "servocenter" => Some(Self::ServoNeutral),
            "lowvoltagecutoff" | "lvc" => Some(Self::LowVoltageCutoff),
            "lvcthreshold" | "lvcvolts" => Some(Self::LvcThreshold),
            "rccarreversing" | "rccar" => Some(Self::RcCarReversing),
            "usehallsensors" | "hall" => Some(Self::UseHallSensors),
            "sinemoderange" | "sinerange" => Some(Self::SineModeRange),
            "dragbrakestrength" | "dragbrake" => Some(Self::DragBrakeStrength),
            _ => None,
        }
    }

    /// Get all available setting names.
    pub fn all_names() -> &'static [&'static str] {
        &[
            "direction",
            "bidirectional",
            "sinusoidal",
            "comp-pwm",
            "variable-pwm",
            "stuck-rotor",
            "timing",
            "pwm-freq",
            "startup-power",
            "motor-kv",
            "motor-poles",
            "brake",
            "stall",
            "beep-volume",
            "telemetry",
            "servo-low",
            "servo-high",
            "servo-neutral",
            "lvc",
            "lvc-threshold",
            "rc-car",
            "hall",
            "sine-range",
            "drag-brake",
        ]
    }

    /// Returns true if this setting takes a boolean value.
    pub fn is_bool(&self) -> bool {
        matches!(
            self,
            Self::DirectionReversed
                | Self::Bidirectional
                | Self::SinusoidalStartup
                | Self::ComplementaryPwm
                | Self::VariablePwm
                | Self::StuckRotorProtection
                | Self::BrakeOnStop
                | Self::StallProtection
                | Self::Telemetry30ms
                | Self::LowVoltageCutoff
                | Self::RcCarReversing
                | Self::UseHallSensors
        )
    }

    /// Get description and valid range for this setting.
    pub fn description(&self) -> &'static str {
        match self {
            Self::DirectionReversed => "Motor direction (on/off)",
            Self::Bidirectional => "3D/bidirectional mode (on/off)",
            Self::SinusoidalStartup => "Sinusoidal startup (on/off)",
            Self::ComplementaryPwm => "Complementary PWM (on/off)",
            Self::VariablePwm => "Variable PWM frequency (on/off)",
            Self::StuckRotorProtection => "Stuck rotor protection (on/off)",
            Self::TimingAdvance => "Motor timing (0-4: 0°/7.5°/15°/22.5°/30°)",
            Self::PwmFrequency => "PWM frequency in kHz (16-96)",
            Self::StartupPower => "Startup power percentage (50-150)",
            Self::MotorKv => "Motor KV rating (40-10200)",
            Self::MotorPoles => "Motor pole count (2-36)",
            Self::BrakeOnStop => "Brake on stop (on/off)",
            Self::StallProtection => "Stall protection (on/off)",
            Self::BeepVolume => "Beep volume (0-11)",
            Self::Telemetry30ms => "30ms telemetry output (on/off)",
            Self::ServoLow => "Servo low pulse raw value (0-255)",
            Self::ServoHigh => "Servo high pulse raw value (0-255)",
            Self::ServoNeutral => "Servo neutral raw value (0-255)",
            Self::LowVoltageCutoff => "Low voltage cutoff enable (on/off)",
            Self::LvcThreshold => "LVC threshold raw (0-255, volts = 2.5 + val*0.1)",
            Self::RcCarReversing => "RC car reversing mode (on/off)",
            Self::UseHallSensors => "Use Hall sensors (on/off)",
            Self::SineModeRange => "Sine mode range percent (5-25)",
            Self::DragBrakeStrength => "Drag brake strength (1-10)",
        }
    }
}

/// Parse a value string into bool.
pub fn parse_bool(s: &str) -> Option<bool> {
    match s.to_lowercase().as_str() {
        "on" | "true" | "yes" | "1" | "enabled" => Some(true),
        "off" | "false" | "no" | "0" | "disabled" => Some(false),
        _ => None,
    }
}

/// Apply a setting to the config.
///
/// Returns Ok(()) on success, Err with message on invalid value.
pub fn apply_setting(config: &mut Am32Config, setting: Setting, value: &str) -> Result<(), &'static str> {
    match setting {
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
            config.set_variable_pwm(parse_bool(value).ok_or("expected on/off")?);
        }
        Setting::StuckRotorProtection => {
            config.set_stuck_rotor_protection(parse_bool(value).ok_or("expected on/off")?);
        }
        Setting::TimingAdvance => {
            let v: u8 = value.parse().map_err(|_| "expected 0-4")?;
            if v > 4 {
                return Err("timing must be 0-4");
            }
            config.set_timing_advance(MotorTiming::from_byte(v));
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
        Setting::BrakeOnStop => {
            config.set_brake_on_stop(parse_bool(value).ok_or("expected on/off")?);
        }
        Setting::StallProtection => {
            config.set_stall_protection(parse_bool(value).ok_or("expected on/off")?);
        }
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
        Setting::LowVoltageCutoff => {
            config.set_low_voltage_cutoff(parse_bool(value).ok_or("expected on/off")?);
        }
        Setting::LvcThreshold => {
            let v: u8 = value.parse().map_err(|_| "expected 0-255")?;
            config.set_lvc_threshold(v);
        }
        Setting::RcCarReversing => {
            config.set_rc_car_reversing(parse_bool(value).ok_or("expected on/off")?);
        }
        Setting::UseHallSensors => {
            config.set_use_hall_sensors(parse_bool(value).ok_or("expected on/off")?);
        }
        Setting::SineModeRange => {
            let v: u8 = value.parse().map_err(|_| "expected 5-25")?;
            if !(5..=25).contains(&v) {
                return Err("sine mode range must be 5-25");
            }
            config.set_sine_mode_range(v);
        }
        Setting::DragBrakeStrength => {
            let v: u8 = value.parse().map_err(|_| "expected 1-10")?;
            if !(1..=10).contains(&v) {
                return Err("drag brake strength must be 1-10");
            }
            config.set_drag_brake_strength(v);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_config() {
        // Sample config from user's ESC
        let mut raw = [0xFF; 256];
        raw[0x00] = 0x01; // boot byte
        raw[0x01] = 0x01; // eeprom version
        raw[0x02] = 0x01; // bootloader version
        raw[0x03] = 0x02; // fw major
        raw[0x04] = 0x0C; // fw minor (12)
        raw[0x05..0x11].copy_from_slice(b"F4A 4in1    "); // name
        raw[0x17] = 0x02; // timing (15°)
        raw[0x18] = 0x30; // pwm freq (48 kHz)
        raw[0x19] = 0x64; // startup power (100%)
        raw[0x1A] = 0x16; // motor kv raw (22 * 40 = 880)
        raw[0x1B] = 0x0E; // motor poles (14)
        raw[0x1E] = 0x05; // beep volume

        let config = Am32Config::from_bytes(raw);

        assert_eq!(config.name(), "F4A 4in1");
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
        assert_eq!(Setting::parse("direction"), Some(Setting::DirectionReversed));
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
}
