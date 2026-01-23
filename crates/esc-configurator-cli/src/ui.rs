//! TUI (Terminal User Interface) components.
//!
//! This module provides interactive prompts and progress indicators
//! using the dialoguer crate.

use anyhow::Result;
use dialoguer::{Confirm, Input, Select, theme::ColorfulTheme};
use esc_protocol::am32_config::{Am32Config, MotorTiming, Setting};

/// Prompt user to select a serial port from available ports.
pub fn select_port(ports: &[String]) -> Result<Option<String>> {
    if ports.is_empty() {
        return Ok(None);
    }

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select serial port")
        .items(ports)
        .default(0)
        .interact_opt()?;

    Ok(selection.map(|i| ports[i].clone()))
}

/// Confirm a destructive operation.
pub fn confirm_operation(message: &str) -> Result<bool> {
    let confirmed = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(message)
        .default(false)
        .interact()?;

    Ok(confirmed)
}

// =============================================================================
// Interactive Configuration Mode
// =============================================================================

/// Main menu options for interactive mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MainMenuAction {
    ViewSettings,
    EditSettings,
    BackupConfig,
    RestoreConfig,
    SaveToEsc,
    Exit,
}

/// Setting categories for grouped display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingCategory {
    Motor,
    Pwm,
    Startup,
    Protection,
    Braking,
    Servo,
    Other,
}

impl SettingCategory {
    /// Get all categories.
    pub const fn all() -> &'static [SettingCategory] {
        &[
            SettingCategory::Motor,
            SettingCategory::Pwm,
            SettingCategory::Startup,
            SettingCategory::Protection,
            SettingCategory::Braking,
            SettingCategory::Servo,
            SettingCategory::Other,
        ]
    }

    /// Get display name for category.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Motor => "Motor",
            Self::Pwm => "PWM",
            Self::Startup => "Startup",
            Self::Protection => "Protection",
            Self::Braking => "Braking",
            Self::Servo => "Servo/Input",
            Self::Other => "Other",
        }
    }

    /// Get settings in this category.
    pub fn settings(self) -> &'static [(Setting, &'static str)] {
        match self {
            Self::Motor => &[
                (Setting::DirectionReversed, "direction"),
                (Setting::MotorKv, "motor-kv"),
                (Setting::MotorPoles, "motor-poles"),
                (Setting::TimingAdvance, "timing"),
            ],
            Self::Pwm => &[
                (Setting::PwmFrequency, "pwm-freq"),
                (Setting::ComplementaryPwm, "comp-pwm"),
                (Setting::VariablePwm, "variable-pwm"),
            ],
            Self::Startup => &[
                (Setting::StartupPower, "startup-power"),
                (Setting::SinusoidalStartup, "sinusoidal"),
            ],
            Self::Protection => &[
                (Setting::StuckRotorProtection, "stuck-rotor"),
                (Setting::StallProtection, "stall"),
                (Setting::LowVoltageCutoff, "lvc"),
                (Setting::LvcThreshold, "lvc-threshold"),
            ],
            Self::Braking => &[
                (Setting::BrakeOnStop, "brake"),
                (Setting::DragBrakeStrength, "drag-brake"),
            ],
            Self::Servo => &[
                (Setting::ServoLow, "servo-low"),
                (Setting::ServoHigh, "servo-high"),
                (Setting::ServoNeutral, "servo-neutral"),
            ],
            Self::Other => &[
                (Setting::BeepVolume, "beep-volume"),
                (Setting::Telemetry30ms, "telemetry"),
                (Setting::RcCarReversing, "rc-car"),
                (Setting::UseHallSensors, "hall"),
                (Setting::SineModeRange, "sine-range"),
                (Setting::Bidirectional, "bidirectional"),
            ],
        }
    }
}

/// Show the main menu and return selected action.
pub fn show_main_menu(esc_name: &str, has_unsaved_changes: bool) -> Result<MainMenuAction> {
    let title = if has_unsaved_changes {
        format!("ESC Configurator - {} [*modified]", esc_name)
    } else {
        format!("ESC Configurator - {}", esc_name)
    };

    println!("\n{title}");
    println!("{}", "=".repeat(title.len()));

    let items = [
        "View Current Settings",
        "Edit Settings",
        "Backup Config to File",
        "Restore Config from File",
        "Save Changes to ESC",
        "Exit",
    ];

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select action")
        .items(items)
        .default(0)
        .interact()?;

    Ok(match selection {
        0 => MainMenuAction::ViewSettings,
        1 => MainMenuAction::EditSettings,
        2 => MainMenuAction::BackupConfig,
        3 => MainMenuAction::RestoreConfig,
        4 => MainMenuAction::SaveToEsc,
        _ => MainMenuAction::Exit,
    })
}

/// Show category selection menu.
pub fn select_category() -> Result<Option<SettingCategory>> {
    let categories = SettingCategory::all();
    let mut items: Vec<String> = categories.iter().map(|c| c.name().to_string()).collect();
    items.push("Back".to_string());

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select category")
        .items(items)
        .default(0)
        .interact()?;

    if selection >= categories.len() {
        Ok(None) // Back selected
    } else {
        Ok(Some(categories[selection]))
    }
}

/// Show setting selection menu within a category, displaying current values.
pub fn select_setting_in_category(
    category: SettingCategory,
    config: &Am32Config,
) -> Result<Option<(Setting, &'static str)>> {
    let settings = category.settings();

    let mut items: Vec<String> = settings
        .iter()
        .map(|(setting, name)| {
            let value = get_setting_display_value(*setting, config);
            format!("{}: {}", name, value)
        })
        .collect();
    items.push("Back".to_string());

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt(format!("{} Settings", category.name()))
        .items(items)
        .default(0)
        .interact()?;

    if selection >= settings.len() {
        Ok(None) // Back selected
    } else {
        Ok(Some(settings[selection]))
    }
}

/// Get display value for a setting from config.
pub fn get_setting_display_value(setting: Setting, config: &Am32Config) -> String {
    match setting {
        Setting::DirectionReversed => {
            if config.direction_reversed() {
                "Reversed"
            } else {
                "Normal"
            }
            .to_string()
        }
        Setting::Bidirectional => on_off(config.bidirectional()),
        Setting::SinusoidalStartup => on_off(config.sinusoidal_startup()),
        Setting::ComplementaryPwm => on_off(config.complementary_pwm()),
        Setting::VariablePwm => on_off(config.variable_pwm()),
        Setting::StuckRotorProtection => on_off(config.stuck_rotor_protection()),
        Setting::TimingAdvance => format!("{}", config.timing_advance()),
        Setting::PwmFrequency => format!("{} kHz", config.pwm_frequency_khz()),
        Setting::StartupPower => format!("{}%", config.startup_power()),
        Setting::MotorKv => format!("{}", config.motor_kv()),
        Setting::MotorPoles => format!("{}", config.motor_poles()),
        Setting::BrakeOnStop => on_off(config.brake_on_stop()),
        Setting::StallProtection => on_off(config.stall_protection()),
        Setting::BeepVolume => format!("{}/11", config.beep_volume()),
        Setting::Telemetry30ms => on_off(config.telemetry_30ms()),
        Setting::ServoLow => format!("{} us (raw {})", config.servo_low_us(), config.servo_low()),
        Setting::ServoHigh => {
            format!("{} us (raw {})", config.servo_high_us(), config.servo_high())
        }
        Setting::ServoNeutral => format!(
            "{} us (raw {})",
            config.servo_neutral_us(),
            config.servo_neutral()
        ),
        Setting::LowVoltageCutoff => on_off(config.low_voltage_cutoff()),
        Setting::LvcThreshold => format!(
            "{:.1}V (raw {})",
            config.lvc_threshold_volts(),
            config.lvc_threshold()
        ),
        Setting::RcCarReversing => on_off(config.rc_car_reversing()),
        Setting::UseHallSensors => on_off(config.use_hall_sensors()),
        Setting::SineModeRange => format!("{}%", config.sine_mode_range()),
        Setting::DragBrakeStrength => format!("{}/10", config.drag_brake_strength()),
    }
}

fn on_off(value: bool) -> String {
    if value { "On" } else { "Off" }.to_string()
}

/// Edit a boolean setting.
pub fn edit_bool_setting(setting_name: &str, current: bool) -> Result<Option<bool>> {
    let items = ["Off", "On"];
    let default_idx = if current { 1 } else { 0 };

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt(format!("{} (current: {})", setting_name, items[default_idx]))
        .items(items)
        .default(default_idx)
        .interact_opt()?;

    Ok(selection.map(|i| i == 1))
}

/// Edit a direction setting (special case for reversed).
pub fn edit_direction_setting(current_reversed: bool) -> Result<Option<bool>> {
    let items = ["Normal", "Reversed"];
    let default_idx = if current_reversed { 1 } else { 0 };

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt(format!("Motor Direction (current: {})", items[default_idx]))
        .items(items)
        .default(default_idx)
        .interact_opt()?;

    Ok(selection.map(|i| i == 1))
}

/// Edit motor timing setting.
pub fn edit_timing_setting(current: MotorTiming) -> Result<Option<MotorTiming>> {
    let items = ["0°", "7.5°", "15°", "22.5°", "30°"];
    let default_idx = current as usize;

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt(format!("Motor Timing (current: {})", current))
        .items(items)
        .default(default_idx)
        .interact_opt()?;

    Ok(selection.map(|i| MotorTiming::from_byte(i as u8)))
}

/// Edit a numeric u8 setting with validation.
pub fn edit_u8_setting(setting_name: &str, current: u8, min: u8, max: u8) -> Result<Option<u8>> {
    let prompt = format!("{} ({}-{}, current: {})", setting_name, min, max, current);

    let input: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt(&prompt)
        .default(current.to_string())
        .validate_with(move |input: &String| -> std::result::Result<(), String> {
            match input.parse::<u8>() {
                Ok(v) if (min..=max).contains(&v) => Ok(()),
                Ok(_) => Err(format!("Value must be between {} and {}", min, max)),
                Err(_) => Err("Please enter a valid number".to_string()),
            }
        })
        .interact_text()?;

    Ok(input.parse().ok())
}

/// Edit motor KV setting (u16).
pub fn edit_motor_kv_setting(current: u16) -> Result<Option<u16>> {
    let prompt = format!("Motor KV (40-10200, current: {})", current);

    let input: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt(&prompt)
        .default(current.to_string())
        .validate_with(|input: &String| -> std::result::Result<(), String> {
            match input.parse::<u16>() {
                Ok(v) if (40..=10200).contains(&v) => Ok(()),
                Ok(_) => Err("Value must be between 40 and 10200".to_string()),
                Err(_) => Err("Please enter a valid number".to_string()),
            }
        })
        .interact_text()?;

    Ok(input.parse().ok())
}

/// Prompt for backup file path.
pub fn prompt_backup_path() -> Result<Option<String>> {
    let input: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Backup file path")
        .default("esc_backup.bin".to_string())
        .interact_text()?;

    if input.is_empty() {
        Ok(None)
    } else {
        Ok(Some(input))
    }
}

/// Prompt for restore file path.
pub fn prompt_restore_path() -> Result<Option<String>> {
    let input: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Restore file path")
        .interact_text()?;

    if input.is_empty() {
        Ok(None)
    } else {
        Ok(Some(input))
    }
}

/// Confirm save with unsaved changes.
pub fn confirm_exit_with_unsaved() -> Result<bool> {
    Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("You have unsaved changes. Save before exiting?")
        .default(true)
        .interact()
        .map_err(Into::into)
}

/// Confirm overwriting configuration.
pub fn confirm_restore() -> Result<bool> {
    Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("This will overwrite current settings. Continue?")
        .default(false)
        .interact()
        .map_err(Into::into)
}
