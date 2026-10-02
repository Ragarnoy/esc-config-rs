//! CLI commands implementation.
//!
//! This module implements the CLI commands:
//! - config: Interactive configuration mode
//! - flash: Flash firmware to ESC
//! - read: Read configuration from ESC
//! - verify: Verify firmware on ESC
//! - backup: Backup current firmware/config

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use esc_protocol::am32_config::{Am32Config, Am32ConfigData, Setting, apply_setting};
use esc_protocol::config::ConfigBlock;
use esc_protocol::device::{DeviceInfo, FirmwareType};
use esc_protocol::ihex::{Firmware, load_firmware};

use crate::relay::Relay;
use crate::ui::{
    self, MainMenuAction, MelodyAction, confirm_exit_with_unsaved, confirm_restore,
    edit_bool_setting, edit_direction_setting, edit_motor_kv_setting, edit_timing_setting,
    edit_u8_setting, get_setting_display_value, prompt_backup_path, prompt_firmware_path,
    prompt_melody_download_path, prompt_melody_upload_path, prompt_restore_path, select_category,
    select_setting_in_category, show_main_menu, show_melody_menu,
};

/// Format device info for display.
fn format_device(info: DeviceInfo) -> String {
    let firmware_name = match info.firmware {
        Some(FirmwareType::Am32) => "AM32",
        None => "Unknown",
    };
    format!("{} (0x{:04X})", firmware_name, info.device_id)
}

// =============================================================================
// Interactive Configuration Mode
// =============================================================================

/// Run interactive configuration mode.
///
/// If device is None, prompts user to select a port.
pub fn interactive_config(device: Option<&str>) -> Result<()> {
    // Resolve device - either from argument or prompt
    let device = match device {
        Some(d) => d.to_string(),
        None => {
            let ports = crate::relay::list_ports()?;
            if ports.is_empty() {
                bail!("No serial ports found. Connect the relay MCU and try again.");
            }
            match ui::select_port(&ports)? {
                Some(port) => port,
                None => {
                    println!("No port selected. Exiting.");
                    return Ok(());
                }
            }
        }
    };

    // Initial connection to read config
    println!("Connecting to ESC via {}...", device);

    let mut relay = Relay::open(&device)?;
    relay.connect()?;

    let device_info = relay.session().device().expect("should be connected");
    println!("Connected: {}", format_device(*device_info));

    // Check if this is AM32
    if device_info.firmware != Some(FirmwareType::Am32) {
        relay.disconnect()?;
        bail!("Interactive config only supported for AM32 ESCs.");
    }

    if let Ok(name) = relay.read_esc_name()
        && !name.is_empty()
    {
        println!("ESC firmware: {name}");
    }

    println!("Reading configuration...");
    let config_block = relay.read_config()?;

    // Disconnect - the bootloader will timeout anyway during menu browsing
    // We'll reconnect when we need to save
    relay.disconnect()?;
    drop(relay);

    // Parse as AM32 config
    let mut raw = [0u8; 256];
    raw.copy_from_slice(config_block.as_bytes());
    let mut config = Am32Config::from_bytes(raw);
    let original_config = config.clone();

    let (fw_major, fw_minor) = config.firmware_version();
    let esc_name = format!("AM32 v{}.{}", fw_major, fw_minor);
    let mut has_unsaved_changes = false;

    // Main menu loop
    loop {
        match show_main_menu(&esc_name, has_unsaved_changes)? {
            MainMenuAction::ViewSettings => {
                println!("\n{}", config);
            }
            MainMenuAction::EditSettings => {
                if edit_settings_loop(&mut config)? == Some(true) {
                    has_unsaved_changes = config.as_bytes() != original_config.as_bytes();
                }
            }
            MainMenuAction::FlashFirmware => {
                if let Some(path) = prompt_firmware_path()? {
                    if Path::new(&path).exists() {
                        if ui::confirm_operation(
                            "Flashing firmware will erase current firmware. Continue?",
                        )? {
                            match flash_firmware(&device, &path) {
                                Ok(()) => println!("Firmware flashed successfully!"),
                                Err(e) => println!("Flash failed: {}", e),
                            }
                        }
                    } else {
                        println!("File not found: {}", path);
                    }
                }
            }
            MainMenuAction::VerifyFirmware => {
                if let Some(path) = prompt_firmware_path()? {
                    if Path::new(&path).exists() {
                        match verify_firmware(&device, &path) {
                            Ok(()) => {}
                            Err(e) => println!("Verification failed: {}", e),
                        }
                    } else {
                        println!("File not found: {}", path);
                    }
                }
            }
            MainMenuAction::Melody => {
                handle_melody_menu(&device)?;
            }
            MainMenuAction::BackupConfig => {
                if let Some(path) = prompt_backup_path()? {
                    match fs::write(&path, config.as_bytes()) {
                        Ok(()) => println!("Configuration backed up to: {}", path),
                        Err(e) => println!("Failed to save backup: {}", e),
                    }
                }
            }
            MainMenuAction::RestoreConfig => {
                if let Some(path) = prompt_restore_path()? {
                    match fs::read(&path) {
                        Ok(data) if data.len() == 256 => {
                            if confirm_restore()? {
                                let mut new_raw = [0u8; 256];
                                new_raw.copy_from_slice(&data);
                                config = Am32Config::from_bytes(new_raw);
                                has_unsaved_changes =
                                    config.as_bytes() != original_config.as_bytes();
                                println!("Configuration loaded from: {}", path);
                            }
                        }
                        Ok(data) => {
                            println!(
                                "Invalid backup file: expected 256 bytes, got {}",
                                data.len()
                            );
                        }
                        Err(e) => println!("Failed to read file: {}", e),
                    }
                }
            }
            MainMenuAction::SaveToEsc => {
                if has_unsaved_changes {
                    // Reconnect to ESC (bootloader has likely timed out)
                    save_config_to_esc(&device, &config)?;
                    has_unsaved_changes = false;
                } else {
                    println!("No changes to save.");
                }
            }
            MainMenuAction::Exit => {
                if has_unsaved_changes && confirm_exit_with_unsaved()? {
                    // Reconnect to ESC (bootloader has likely timed out)
                    save_config_to_esc(&device, &config)?;
                }
                break;
            }
        }
    }

    println!("Done.");
    Ok(())
}

/// Handle the melody sub-menu.
fn handle_melody_menu(device: &str) -> Result<()> {
    loop {
        match show_melody_menu()? {
            MelodyAction::ViewMelody => {
                // Read and display current melody from ESC
                match read_melody_from_esc(device) {
                    Ok(data) => {
                        if data[0] == esc_protocol::MELODY_EMPTY {
                            println!("\nNo melody configured on ESC.");
                        } else if let Some(melody) = esc_protocol::Melody::from_bytes(&data) {
                            println!("\nCurrent melody: {} notes", melody.note_count());
                            println!("{melody}");
                        } else {
                            println!("\nFailed to parse melody data.");
                        }
                    }
                    Err(e) => println!("Failed to read melody: {}", e),
                }
            }
            MelodyAction::UploadMelody => {
                if let Some(path) = prompt_melody_upload_path()? {
                    if Path::new(&path).exists() {
                        match melody_upload(device, &path) {
                            Ok(()) => println!("Melody uploaded successfully!"),
                            Err(e) => println!("Upload failed: {}", e),
                        }
                    } else {
                        println!("File not found: {}", path);
                    }
                }
            }
            MelodyAction::DownloadMelody => {
                if let Some(path) = prompt_melody_download_path()? {
                    match melody_download(device, &path) {
                        Ok(()) => {}
                        Err(e) => println!("Download failed: {}", e),
                    }
                }
            }
            MelodyAction::ClearMelody => {
                if ui::confirm_operation("Clear melody from ESC?")? {
                    match melody_clear(device) {
                        Ok(()) => {}
                        Err(e) => println!("Clear failed: {}", e),
                    }
                }
            }
            MelodyAction::Back => break,
        }
    }
    Ok(())
}

/// Read raw melody data from ESC (for viewing in interactive mode).
fn read_melody_from_esc(device: &str) -> Result<[u8; 128]> {
    let mut relay = Relay::open(device)?;
    relay.connect()?;

    let device_info = relay.session().device().expect("should be connected");
    if device_info.firmware != Some(FirmwareType::Am32) {
        relay.disconnect()?;
        bail!("Melody support only available for AM32 ESCs");
    }

    let data = relay.read_melody()?;
    relay.disconnect()?;
    Ok(data)
}

/// Save configuration to ESC by reconnecting and writing.
///
/// Note: We must read the config first to "prime" the connection before writing.
/// The ESC bootloader requires a read operation before it will accept writes
/// on a fresh connection.
fn save_config_to_esc(device: &str, config: &Am32Config) -> Result<()> {
    println!("Reconnecting to ESC...");

    let mut relay = Relay::open(device)?;
    relay.connect()?;

    // Read current config first - this primes the connection for writing.
    // Without this read, the ESC will NACK the write command.
    println!("Preparing for write...");
    let _ = relay.read_config()?;

    println!("Writing configuration...");
    let config_block = ConfigBlock::from_array(*config.as_bytes());
    relay.write_config(&config_block)?;

    relay.disconnect()?;
    println!("Configuration saved successfully!");

    Ok(())
}

/// Edit settings loop - returns Some(true) if changes were made.
fn edit_settings_loop(config: &mut Am32Config) -> Result<Option<bool>> {
    let mut modified = false;

    loop {
        // Select category
        let Some(category) = select_category()? else {
            return Ok(Some(modified));
        };

        // Select setting in category
        loop {
            let Some((setting, name)) = select_setting_in_category(category, config)? else {
                break; // Back to category selection
            };

            // Edit the setting
            if edit_single_setting(config, setting, name)? {
                modified = true;
            }
        }
    }
}

/// Edit a single setting. Returns true if the value was changed.
fn edit_single_setting(config: &mut Am32Config, setting: Setting, name: &str) -> Result<bool> {
    match setting {
        // Throttle/Rate settings
        Setting::MaxRateOfChange => {
            if let Some(value) = edit_u8_setting(name, config.max_rate_of_change(), 1, 200)? {
                config.set_max_rate_of_change(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::MinDuty => {
            if let Some(value) = edit_u8_setting(name, config.min_duty(), 0, 50)? {
                config.set_min_duty(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::DisableStickCalibration => {
            if let Some(value) = edit_bool_setting(name, config.disable_stick_calibration())? {
                config.set_disable_stick_calibration(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::AbsoluteVoltage => {
            if let Some(value) = edit_u8_setting(name, config.absolute_voltage(), 0, 255)? {
                config.set_absolute_voltage(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::CurrentLimitP => {
            if let Some(value) = edit_u8_setting(name, config.current_limit_p(), 0, 255)? {
                config.set_current_limit_p(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::CurrentLimitI => {
            if let Some(value) = edit_u8_setting(name, config.current_limit_i(), 0, 255)? {
                config.set_current_limit_i(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::CurrentLimitD => {
            if let Some(value) = edit_u8_setting(name, config.current_limit_d(), 0, 255)? {
                config.set_current_limit_d(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::ActiveBrakeStrength => {
            if let Some(value) = edit_u8_setting(name, config.active_brake_strength(), 1, 10)? {
                config.set_active_brake_strength(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        // Motor settings
        Setting::DirectionReversed => {
            if let Some(value) = edit_direction_setting(config.direction_reversed())? {
                config.set_direction_reversed(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::Bidirectional => {
            if let Some(value) = edit_bool_setting(name, config.bidirectional())? {
                config.set_bidirectional(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::SinusoidalStartup => {
            if let Some(value) = edit_bool_setting(name, config.sinusoidal_startup())? {
                config.set_sinusoidal_startup(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::ComplementaryPwm => {
            if let Some(value) = edit_bool_setting(name, config.complementary_pwm())? {
                config.set_complementary_pwm(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::VariablePwm => {
            if let Some(value) = edit_bool_setting(name, config.variable_pwm())? {
                config.set_variable_pwm(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::StuckRotorProtection => {
            if let Some(value) = edit_bool_setting(name, config.stuck_rotor_protection())? {
                config.set_stuck_rotor_protection(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::TimingAdvance => {
            if let Some(value) = edit_timing_setting(config.timing_advance())? {
                config.set_timing_advance(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::PwmFrequency => {
            if let Some(value) = edit_u8_setting(name, config.pwm_frequency_khz(), 16, 96)? {
                config.set_pwm_frequency_khz(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::StartupPower => {
            if let Some(value) = edit_u8_setting(name, config.startup_power(), 50, 150)? {
                config.set_startup_power(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::MotorKv => {
            if let Some(value) = edit_motor_kv_setting(config.motor_kv())? {
                config.set_motor_kv(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::MotorPoles => {
            if let Some(value) = edit_u8_setting(name, config.motor_poles(), 2, 36)? {
                config.set_motor_poles(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::BrakeOnStop => {
            if let Some(value) = edit_bool_setting(name, config.brake_on_stop())? {
                config.set_brake_on_stop(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::StallProtection => {
            if let Some(value) = edit_bool_setting(name, config.stall_protection())? {
                config.set_stall_protection(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::BeepVolume => {
            if let Some(value) = edit_u8_setting(name, config.beep_volume(), 0, 11)? {
                config.set_beep_volume(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::Telemetry30ms => {
            if let Some(value) = edit_bool_setting(name, config.telemetry_30ms())? {
                config.set_telemetry_30ms(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::ServoLow => {
            if let Some(value) = edit_u8_setting(name, config.servo_low(), 0, 255)? {
                config.set_servo_low(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::ServoHigh => {
            if let Some(value) = edit_u8_setting(name, config.servo_high(), 0, 255)? {
                config.set_servo_high(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::ServoNeutral => {
            if let Some(value) = edit_u8_setting(name, config.servo_neutral(), 0, 255)? {
                config.set_servo_neutral(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::LowVoltageCutoff => {
            if let Some(value) = edit_bool_setting(name, config.low_voltage_cutoff())? {
                config.set_low_voltage_cutoff(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::LvcThreshold => {
            if let Some(value) = edit_u8_setting(name, config.lvc_threshold(), 0, 255)? {
                config.set_lvc_threshold(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::RcCarReversing => {
            if let Some(value) = edit_bool_setting(name, config.rc_car_reversing())? {
                config.set_rc_car_reversing(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::UseHallSensors => {
            if let Some(value) = edit_bool_setting(name, config.use_hall_sensors())? {
                config.set_use_hall_sensors(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::SineModeRange => {
            if let Some(value) = edit_u8_setting(name, config.sine_mode_range(), 5, 25)? {
                config.set_sine_mode_range(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::DragBrakeStrength => {
            if let Some(value) = edit_u8_setting(name, config.drag_brake_strength(), 1, 10)? {
                config.set_drag_brake_strength(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::RunningBrakeStrength => {
            if let Some(value) = edit_u8_setting(name, config.running_brake_strength(), 1, 10)? {
                config.set_running_brake_strength(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::AutoTiming => {
            if let Some(value) = edit_bool_setting(name, config.auto_timing())? {
                config.set_auto_timing(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::TemperatureLimit => {
            if let Some(value) = edit_u8_setting(name, config.temperature_limit(), 70, 142)? {
                config.set_temperature_limit(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::CurrentLimit => {
            if let Some(value) = edit_u8_setting(name, config.current_limit(), 0, 255)? {
                config.set_current_limit(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::SineModePower => {
            if let Some(value) = edit_u8_setting(name, config.sine_mode_power(), 1, 10)? {
                config.set_sine_mode_power(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::SignalType => {
            if let Some(value) = edit_u8_setting(name, config.signal_type() as u8, 0, 2)? {
                config.set_signal_type(esc_protocol::SignalType::from_byte(value));
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
        Setting::ServoDeadband => {
            if let Some(value) = edit_u8_setting(name, config.servo_deadband(), 0, 255)? {
                config.set_servo_deadband(value);
                println!(
                    "Set {} = {}",
                    name,
                    get_setting_display_value(setting, config)
                );
                return Ok(true);
            }
        }
    }
    Ok(false)
}

// =============================================================================
// Original Commands
// =============================================================================

/// Read configuration from ESC and display info.
pub fn read_config(device: &str, output: Option<&str>) -> Result<()> {
    println!("Connecting to ESC via {device}...");

    let mut relay = Relay::open(device)?;
    relay.connect()?;

    let device_info = relay.session().device().expect("should be connected");
    println!("Connected: {}", format_device(*device_info));

    println!("Reading configuration...");
    let config = relay.read_config()?;

    // Display config as hex dump
    println!("\nConfiguration block (256 bytes, encrypted):");
    print_hex_dump(config.as_bytes());

    // Save to file if requested
    if let Some(path) = output {
        fs::write(path, config.as_bytes())
            .with_context(|| format!("failed to write config to {path}"))?;
        println!("\nConfig saved to: {path}");
    }

    // Read device ID block
    println!("\nReading device ID...");
    let device_id = relay.read_device_id()?;
    println!("Device ID block:");
    print_hex_line(&device_id);

    // Disconnect cleanly
    relay.disconnect()?;
    println!("\nDisconnected.");

    Ok(())
}

/// Flash firmware hex file to ESC.
///
/// # Safety
/// This function handles the boot byte (safety bit) to prevent bricking:
/// 1. Clears boot byte before flash (ESC won't boot corrupted firmware)
/// 2. Restores boot byte after successful flash
/// 3. Attempts to restore boot byte even on failure
pub fn flash_firmware(device: &str, hex_file: &str) -> Result<()> {
    use indicatif::{ProgressBar, ProgressStyle};

    // Validate file exists
    if !Path::new(hex_file).exists() {
        bail!("hex file not found: {hex_file}");
    }

    // Load and parse firmware
    println!("Loading firmware from {hex_file}...");
    let content =
        fs::read_to_string(hex_file).with_context(|| format!("failed to read {hex_file}"))?;
    let firmware =
        load_firmware(&content).with_context(|| format!("failed to parse {hex_file}"))?;
    let binary = firmware.to_binary();

    println!(
        "Firmware size: {} bytes ({:.1} KB)",
        binary.len(),
        binary.len() as f64 / 1024.0
    );
    if let Some(min) = firmware.min_address() {
        println!(
            "Address range: 0x{:08X} - 0x{:08X}",
            min,
            firmware.max_address().unwrap_or(min)
        );
    }

    // Connect to ESC
    println!("\nConnecting to ESC via {device}...");
    let mut relay = Relay::open(device)?;
    relay.connect()?;

    let device_info = relay.session().device().expect("should be connected");
    println!("Connected: {}", format_device(*device_info));

    // Check if this is AM32
    if device_info.firmware != Some(FirmwareType::Am32) {
        relay.disconnect()?;
        bail!("Firmware flashing only supported for AM32 ESCs");
    }

    // Read ESC firmware name and verify MCU type matches HEX file
    match relay.read_esc_name() {
        Ok(esc_name) if !esc_name.is_empty() => {
            println!("ESC firmware: {esc_name}");
            // Extract MCU type from ESC name (last segment after '_')
            if let Some(esc_mcu) = esc_name.rsplit('_').next() {
                // Extract MCU type from HEX filename for comparison
                let hex_basename = Path::new(hex_file)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("");
                // HEX filenames look like "AM32_F4A_4IN1_F421_2.20"
                // Strip version suffix (last segment with a dot) then take MCU type
                let hex_parts: Vec<&str> = hex_basename.split('_').collect();
                let hex_mcu = hex_parts
                    .iter()
                    .rev()
                    .find(|part| !part.contains('.'))
                    .copied()
                    .unwrap_or("");

                if !hex_mcu.is_empty() && !hex_mcu.eq_ignore_ascii_case(esc_mcu) {
                    relay.disconnect()?;
                    bail!(
                        "MCU type mismatch: ESC has '{}' but firmware is for '{}'. \
                         Flashing wrong firmware can brick the ESC!",
                        esc_mcu,
                        hex_mcu
                    );
                }
            }
        }
        Ok(_) => println!("ESC firmware name: (empty)"),
        Err(e) => println!("Could not read ESC name: {e}"),
    }

    // Step 1: Clear boot byte (SAFETY)
    println!("\nPreparing ESC for flash...");
    let mut eeprom = relay.read_eeprom()?;
    let original_boot_byte = eeprom[0];
    println!("  Boot byte: 0x{:02X}", original_boot_byte);

    eeprom[0] = 0x00; // Clear boot byte - ESC won't boot if flash fails
    relay.write_eeprom(&eeprom)?;
    println!("  Boot byte cleared (safety mode)");

    // Step 2: Flash firmware in 128-byte chunks
    let chunk_size = 128usize;
    let total_chunks = binary.len().div_ceil(chunk_size);
    let max_retries = 8;
    let firmware_start: u16 = 0x1000; // AM32 firmware starts at 4KB (after bootloader)

    let pb = ProgressBar::new(total_chunks as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} chunks ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );

    // Step 2a: Erase flash pages before writing
    let page_size: u16 = 1024; // AM32 uses 1KB pages
    let first_page = firmware_start / page_size;
    let last_page = (firmware_start + binary.len() as u16 - 1) / page_size;
    let num_pages = last_page - first_page + 1;
    println!(
        "\nErasing {} pages (0x{:04X}-0x{:04X})...",
        num_pages,
        first_page * page_size,
        (last_page + 1) * page_size - 1
    );
    for page in first_page..=last_page {
        let page_addr = page * page_size;
        relay.set_address(page_addr)?;
        relay.erase_page()?;
    }
    println!("  Pages erased");

    println!(
        "\nFlashing {} chunks to 0x{:04X}...",
        total_chunks, firmware_start
    );

    for chunk_idx in 0..total_chunks {
        let start = chunk_idx * chunk_size;
        let end = (start + chunk_size).min(binary.len());
        let chunk = &binary[start..end];
        let address = firmware_start + (chunk_idx * chunk_size) as u16;

        // Retry loop
        let mut retries = 0;
        loop {
            match relay.write_chunk(address, chunk) {
                Ok(()) => break,
                Err(e) if retries < max_retries => {
                    retries += 1;
                    pb.println(format!(
                        "  Retry {}/{} at 0x{:04X}: {}",
                        retries, max_retries, address, e
                    ));
                }
                Err(e) => {
                    // CRITICAL: Try to restore boot byte before failing
                    pb.finish_with_message("FAILED");
                    println!("\n  Attempting to restore boot byte...");
                    eeprom[0] = original_boot_byte;
                    if relay.write_eeprom(&eeprom).is_ok() {
                        println!("  Boot byte restored");
                    } else {
                        println!("  WARNING: Could not restore boot byte!");
                    }
                    relay.disconnect().ok();
                    bail!(
                        "Flash failed at 0x{:04X} after {} retries: {}",
                        address,
                        retries,
                        e
                    );
                }
            }
        }
        pb.inc(1);
    }
    pb.finish_with_message("done");

    // Step 3: Restore boot byte
    // Preserve the original EEPROM (user settings), only re-enable boot.
    println!("\nFinalizing...");
    eeprom[0] = 0x01; // Enable boot
    relay.write_eeprom(&eeprom)?;
    println!("  Boot byte restored (0x01)");

    // Step 4: Disconnect
    relay.disconnect()?;

    println!("\nFirmware flashed successfully!");
    println!("The ESC will restart with the new firmware.");

    Ok(())
}

/// Verify ESC firmware against hex file.
///
/// Reads the firmware from the ESC and compares it against the hex file.
pub fn verify_firmware(device: &str, hex_file: &str) -> Result<()> {
    use indicatif::{ProgressBar, ProgressStyle};

    if !Path::new(hex_file).exists() {
        bail!("hex file not found: {hex_file}");
    }

    // Load firmware from hex file
    println!("Loading firmware from {hex_file}...");
    let content =
        fs::read_to_string(hex_file).with_context(|| format!("failed to read {hex_file}"))?;
    let firmware =
        load_firmware(&content).with_context(|| format!("failed to parse {hex_file}"))?;
    let binary = firmware.to_binary();

    println!(
        "Firmware size: {} bytes ({:.1} KB)",
        binary.len(),
        binary.len() as f64 / 1024.0
    );

    // Connect to ESC
    println!("\nConnecting to ESC via {device}...");
    let mut relay = Relay::open(device)?;
    relay.connect()?;

    let device_info = relay.session().device().expect("should be connected");
    println!("Connected: {}", format_device(*device_info));

    if device_info.firmware != Some(FirmwareType::Am32) {
        relay.disconnect()?;
        bail!("Firmware verification only supported for AM32 ESCs");
    }

    // Verify in 256-byte chunks (more efficient reads)
    let chunk_size = 256usize;
    let total_chunks = binary.len().div_ceil(chunk_size);
    let firmware_start: u16 = 0x1000; // AM32 firmware starts at 4KB

    let pb = ProgressBar::new(total_chunks as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} chunks")
            .unwrap()
            .progress_chars("#>-"),
    );

    println!("\nVerifying {} chunks...", total_chunks);

    let mut mismatches = 0;
    for chunk_idx in 0..total_chunks {
        let start = chunk_idx * chunk_size;
        let end = (start + chunk_size).min(binary.len());
        let expected = &binary[start..end];
        let address = firmware_start + (chunk_idx * chunk_size) as u16;

        let actual = relay.read_flash(address, expected.len())?;

        if actual != expected {
            mismatches += 1;
            pb.println(format!(
                "  Mismatch at 0x{:04X}: expected {:02X?}..., got {:02X?}...",
                address,
                &expected[..8.min(expected.len())],
                &actual[..8.min(actual.len())]
            ));
        }
        pb.inc(1);
    }

    pb.finish();
    relay.disconnect()?;

    if mismatches > 0 {
        bail!("Verification FAILED: {} chunk(s) did not match", mismatches);
    }

    println!("\nFirmware verified successfully!");
    Ok(())
}

/// Backup ESC configuration to file.
pub fn backup_config(device: &str, output: &str) -> Result<()> {
    println!("Connecting to ESC via {device}...");
    let mut relay = Relay::open(device)?;
    relay.connect()?;

    let device_info = relay.session().device().expect("should be connected");
    println!("Connected: {}", format_device(*device_info));

    println!("Reading configuration...");
    let config = relay.read_config()?;

    fs::write(output, config.as_bytes())
        .with_context(|| format!("failed to write backup to {output}"))?;

    println!("Backup saved to: {output}");

    relay.disconnect()?;
    println!("Disconnected.");

    Ok(())
}

/// Restore ESC configuration from file.
pub fn restore_config(device: &str, input: &str) -> Result<()> {
    if !Path::new(input).exists() {
        bail!("backup file not found: {input}");
    }

    let data = fs::read(input).with_context(|| format!("failed to read {input}"))?;

    if data.len() != 256 {
        bail!(
            "invalid backup file: expected 256 bytes, got {}",
            data.len()
        );
    }

    let config = ConfigBlock::from_bytes(&data);

    println!("Connecting to ESC via {device}...");
    let mut relay = Relay::open(device)?;
    relay.connect()?;

    let device_info = relay.session().device().expect("should be connected");
    println!("Connected: {}", format_device(*device_info));

    println!("Writing configuration...");
    relay.write_config(&config)?;

    println!("Configuration restored from: {input}");

    relay.disconnect()?;
    println!("Disconnected.");

    Ok(())
}

/// Export ESC configuration as TOML (AM32 only).
pub fn export_config(device: &str, output: &str) -> Result<()> {
    println!("Connecting to ESC via {device}...");
    let mut relay = Relay::open(device)?;
    relay.connect()?;

    let device_info = relay.session().device().expect("should be connected");
    println!("Connected: {}", format_device(*device_info));

    if device_info.firmware != Some(FirmwareType::Am32) {
        relay.disconnect()?;
        bail!("TOML export only supported for AM32 ESCs.");
    }

    println!("Reading configuration...");
    let config_block = relay.read_config()?;

    let mut raw = [0u8; 256];
    raw.copy_from_slice(config_block.as_bytes());
    let config = Am32Config::from_bytes(raw);
    let data = Am32ConfigData::from(&config);

    let toml_string =
        toml::to_string_pretty(&data).context("failed to serialize config to TOML")?;

    fs::write(output, &toml_string).with_context(|| format!("failed to write TOML to {output}"))?;

    println!("Configuration exported to: {output}");

    relay.disconnect()?;
    println!("Disconnected.");

    Ok(())
}

/// Import ESC configuration from TOML file (AM32 only).
pub fn import_config(device: &str, input: &str) -> Result<()> {
    if !Path::new(input).exists() {
        bail!("TOML file not found: {input}");
    }

    let toml_string =
        fs::read_to_string(input).with_context(|| format!("failed to read {input}"))?;

    let data: Am32ConfigData = toml::from_str(&toml_string)
        .with_context(|| format!("failed to parse TOML from {input}"))?;

    let config = Am32Config::from(&data);
    let config_block = ConfigBlock::from_array(*config.as_bytes());

    println!("Connecting to ESC via {device}...");
    let mut relay = Relay::open(device)?;
    relay.connect()?;

    let device_info = relay.session().device().expect("should be connected");
    println!("Connected: {}", format_device(*device_info));

    if device_info.firmware != Some(FirmwareType::Am32) {
        relay.disconnect()?;
        bail!("TOML import only supported for AM32 ESCs.");
    }

    println!("Writing configuration...");
    relay.write_config(&config_block)?;

    println!("Configuration imported from: {input}");

    relay.disconnect()?;
    println!("Disconnected.");

    Ok(())
}

/// List available serial ports.
pub fn list_ports() -> Result<()> {
    let ports = crate::relay::list_ports()?;

    if ports.is_empty() {
        println!("No serial ports found.");
    } else {
        println!("Available serial ports:");
        for port in ports {
            println!("  {port}");
        }
    }

    Ok(())
}

/// Display information about an Intel HEX file.
pub fn hex_info(hex_file: &str) -> Result<()> {
    if !Path::new(hex_file).exists() {
        bail!("hex file not found: {hex_file}");
    }

    let content =
        fs::read_to_string(hex_file).with_context(|| format!("failed to read {hex_file}"))?;

    let firmware =
        load_firmware(&content).with_context(|| format!("failed to parse {hex_file}"))?;

    println!("Intel HEX file: {hex_file}");
    println!();
    print_firmware_info(&firmware);

    Ok(())
}

/// Print detailed firmware information.
fn print_firmware_info(firmware: &Firmware) {
    println!("Segments: {}", firmware.segments.len());
    println!(
        "Total size: {} bytes ({:.1} KB)",
        firmware.total_size(),
        firmware.total_size() as f64 / 1024.0
    );

    if let Some(min) = firmware.min_address() {
        println!(
            "Address range: 0x{:08X} - 0x{:08X}",
            min,
            firmware.max_address().unwrap_or(min)
        );
    }

    if let Some(entry) = firmware.entry_point {
        println!("Entry point: 0x{:08X}", entry);
    }

    println!();
    println!("Segment details:");
    for (i, segment) in firmware.segments.iter().enumerate() {
        println!(
            "  [{i}] 0x{:08X} - 0x{:08X} ({} bytes)",
            segment.start_address,
            segment.end_address(),
            segment.data.len()
        );

        // Show first few bytes of each segment
        if !segment.data.is_empty() {
            print!("      First bytes: ");
            for byte in segment.data.iter().take(16) {
                print!("{byte:02X} ");
            }
            if segment.data.len() > 16 {
                print!("...");
            }
            println!();
        }
    }
}

/// Show parsed ESC configuration (human-readable).
pub fn show_config(device: &str) -> Result<()> {
    println!("Connecting to ESC via {device}...");

    let mut relay = Relay::open(device)?;
    relay.connect()?;

    let device_info = relay.session().device().expect("should be connected");
    println!("Connected: {}", format_device(*device_info));

    // Check if this is AM32
    if device_info.firmware != Some(FirmwareType::Am32) {
        relay.disconnect()?;
        bail!("Config parsing only supported for AM32 ESCs. Use 'read' for raw dump.");
    }

    println!("Reading configuration...");
    let config_block = relay.read_config()?;

    // Parse as AM32 config
    let mut raw = [0u8; 256];
    raw.copy_from_slice(config_block.as_bytes());
    let config = Am32Config::from_bytes(raw);

    // Display parsed config
    println!();
    println!("{config}");

    relay.disconnect()?;
    println!("Disconnected.");

    Ok(())
}

/// Set a configuration value on the ESC.
pub fn set_config(device: &str, setting_name: &str, value: &str) -> Result<()> {
    // Parse setting name
    let setting = Setting::parse(setting_name)
        .ok_or_else(|| anyhow::anyhow!(
            "Unknown setting: {setting_name}\nRun 'esc-configurator settings' to see available options."
        ))?;

    println!("Connecting to ESC via {device}...");

    let mut relay = Relay::open(device)?;
    relay.connect()?;

    let device_info = relay.session().device().expect("should be connected");
    println!("Connected: {}", format_device(*device_info));

    // Check if this is AM32
    if device_info.firmware != Some(FirmwareType::Am32) {
        relay.disconnect()?;
        bail!("Config modification only supported for AM32 ESCs.");
    }

    // Read current config
    println!("Reading current configuration...");
    let config_block = relay.read_config()?;

    // Parse as AM32 config
    let mut raw = [0u8; 256];
    raw.copy_from_slice(config_block.as_bytes());
    let mut config = Am32Config::from_bytes(raw);

    // Apply the setting change
    apply_setting(&mut config, setting, value)
        .map_err(|e| anyhow::anyhow!("Invalid value for {setting_name}: {e}"))?;

    println!("Setting {setting_name} = {value}");

    // Write back
    let new_config = ConfigBlock::from_array(*config.as_bytes());
    println!("Writing configuration...");
    relay.write_config(&new_config)?;

    println!("Configuration updated successfully!");

    relay.disconnect()?;
    println!("Disconnected.");

    Ok(())
}

/// List all available settings.
pub fn list_settings() {
    println!("Available ESC settings (AM32):");
    println!();
    println!("{:<20} DESCRIPTION", "SETTING");
    println!("{:<20} -----------", "-------");

    let settings = [
        ("direction", Setting::DirectionReversed),
        ("bidirectional", Setting::Bidirectional),
        ("motor-kv", Setting::MotorKv),
        ("motor-poles", Setting::MotorPoles),
        ("timing", Setting::TimingAdvance),
        ("pwm-freq", Setting::PwmFrequency),
        ("startup-power", Setting::StartupPower),
        ("brake", Setting::BrakeOnStop),
        ("drag-brake", Setting::DragBrakeStrength),
        ("beep-volume", Setting::BeepVolume),
        ("lvc", Setting::LowVoltageCutoff),
        ("lvc-threshold", Setting::LvcThreshold),
        ("stall", Setting::StallProtection),
        ("stuck-rotor", Setting::StuckRotorProtection),
        ("sinusoidal", Setting::SinusoidalStartup),
        ("comp-pwm", Setting::ComplementaryPwm),
        ("variable-pwm", Setting::VariablePwm),
        ("telemetry", Setting::Telemetry30ms),
        ("servo-low", Setting::ServoLow),
        ("servo-high", Setting::ServoHigh),
        ("servo-neutral", Setting::ServoNeutral),
        ("rc-car", Setting::RcCarReversing),
        ("hall", Setting::UseHallSensors),
        ("signal", Setting::SignalType),
        ("sine-range", Setting::SineModeRange),
    ];

    for (name, setting) in settings {
        println!("{:<20} {}", name, setting.description());
    }

    println!();
    println!("Examples:");
    println!("  esc-configurator set -d /dev/ttyACM0 -s motor-kv -v 2200");
    println!("  esc-configurator set -d /dev/ttyACM0 -s direction -v on");
    println!("  esc-configurator set -d /dev/ttyACM0 -s timing -v 2");
}

/// Reset ESC configuration to sensible defaults.
///
/// This writes a default configuration suitable for most ESC setups.
/// Useful after flashing new firmware when EEPROM may have bad data.
pub fn reset_defaults(device: &str) -> Result<()> {
    println!("Connecting to ESC via {device}...");

    let mut relay = Relay::open(device)?;
    relay.connect()?;

    let device_info = relay.session().device().expect("should be connected");
    println!("Connected: {}", format_device(*device_info));

    // Check if this is AM32
    if device_info.firmware != Some(FirmwareType::Am32) {
        relay.disconnect()?;
        bail!("Reset defaults only supported for AM32 ESCs.");
    }

    // Create default config
    println!("Creating default configuration...");
    let config = Am32Config::with_defaults();

    // Show what we're about to write
    println!();
    println!("Default configuration:");
    println!("  Signal Type:       Auto (accepts DShot)");
    println!("  PWM Frequency:     24 kHz");
    println!("  Motor Timing:      22.5° (Auto)");
    println!("  Motor KV:          2000");
    println!("  Motor Poles:       14");
    println!("  Startup Power:     100%");
    println!("  Beep Volume:       5");
    println!("  Stuck Rotor Prot:  On");
    println!("  Stall Protection:  On");
    println!();

    // Write config
    let config_block = ConfigBlock::from_array(*config.as_bytes());
    println!("Writing default configuration...");
    relay.write_config(&config_block)?;

    println!("Configuration reset to defaults!");
    println!();
    println!("The ESC should now accept DShot signals.");
    println!("Power cycle the ESC and test with beep_test or motor_test.");

    relay.disconnect()?;
    println!("Disconnected.");

    Ok(())
}

// ----------------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------------

/// Print a hex dump of data (16 bytes per line).
fn print_hex_dump(data: &[u8]) {
    for (i, chunk) in data.chunks(16).enumerate() {
        print!("{:04X}:  ", i * 16);
        for byte in chunk {
            print!("{byte:02X} ");
        }
        // Pad if short line
        for _ in chunk.len()..16 {
            print!("   ");
        }
        print!(" |");
        for &byte in chunk {
            let c = if byte.is_ascii_graphic() || byte == b' ' {
                byte as char
            } else {
                '.'
            };
            print!("{c}");
        }
        println!("|");
    }
}

/// Print a single line of hex bytes.
fn print_hex_line(data: &[u8]) {
    for byte in data {
        print!("{byte:02X} ");
    }
    println!();
}

// =============================================================================
// Melody Commands
// =============================================================================

/// Download melody from ESC to file.
pub fn melody_download(device: &str, output: &str) -> Result<()> {
    println!("Connecting to ESC via {device}...");
    let mut relay = Relay::open(device)?;
    relay.connect()?;

    let device_info = relay.session().device().expect("should be connected");
    println!("Connected: {}", format_device(*device_info));

    if device_info.firmware != Some(FirmwareType::Am32) {
        relay.disconnect()?;
        bail!("Melody support only available for AM32 ESCs");
    }

    println!("Reading melody data...");
    let data = relay.read_melody()?;

    relay.disconnect()?;

    // Check if melody exists
    if data[0] == esc_protocol::MELODY_EMPTY {
        println!("No melody configured on ESC.");
    } else {
        // Parse and display melody info
        if let Some(melody) = esc_protocol::Melody::from_bytes(&data) {
            println!("Melody: {} notes", melody.note_count());
            println!("  {melody}");
        }
    }

    // Save raw data
    fs::write(output, data).with_context(|| format!("failed to write {output}"))?;
    println!("Saved to: {output}");

    Ok(())
}

/// Upload melody from file to ESC (supports binary or RTTTL format).
pub fn melody_upload(device: &str, input: &str) -> Result<()> {
    if !Path::new(input).exists() {
        bail!("melody file not found: {input}");
    }

    let data = fs::read(input).with_context(|| format!("failed to read {input}"))?;

    // Determine format and convert
    let melody_data: [u8; 128] = if is_rtttl_format(&data) {
        // RTTTL text format
        let text = std::str::from_utf8(&data)
            .with_context(|| "input file is not valid UTF-8")?
            .trim();

        if let Some(name) = esc_protocol::Melody::rtttl_name(text) {
            println!("RTTTL melody: {name}");
        }

        let melody = esc_protocol::Melody::from_rtttl(text)
            .map_err(|e| anyhow::anyhow!("failed to parse RTTTL: {:?}", e))?;

        println!("Uploading melody: {} notes", melody.note_count());
        println!("  {melody}");

        melody.to_bytes()
    } else {
        // Binary format
        if data.len() != 128 {
            bail!(
                "invalid binary melody file: expected 128 bytes, got {}",
                data.len()
            );
        }

        // Parse and display what we're uploading
        if data[0] == esc_protocol::MELODY_EMPTY {
            println!("Warning: file contains empty melody (will clear ESC melody)");
        } else if let Some(melody) = esc_protocol::Melody::from_bytes(&data) {
            println!("Uploading melody: {} notes", melody.note_count());
            println!("  {melody}");
        }

        let mut buf = [0u8; 128];
        buf.copy_from_slice(&data);
        buf
    };

    println!("Connecting to ESC via {device}...");
    let mut relay = Relay::open(device)?;
    relay.connect()?;

    let device_info = relay.session().device().expect("should be connected");
    println!("Connected: {}", format_device(*device_info));

    if device_info.firmware != Some(FirmwareType::Am32) {
        relay.disconnect()?;
        bail!("Melody support only available for AM32 ESCs");
    }

    println!("Writing melody data...");
    relay.write_melody(&melody_data)?;

    relay.disconnect()?;
    println!("Melody uploaded successfully!");

    Ok(())
}

/// Clear melody from ESC.
pub fn melody_clear(device: &str) -> Result<()> {
    println!("Connecting to ESC via {device}...");
    let mut relay = Relay::open(device)?;
    relay.connect()?;

    let device_info = relay.session().device().expect("should be connected");
    println!("Connected: {}", format_device(*device_info));

    if device_info.firmware != Some(FirmwareType::Am32) {
        relay.disconnect()?;
        bail!("Melody support only available for AM32 ESCs");
    }

    println!("Clearing melody...");
    let empty = esc_protocol::Melody::clear_bytes();
    relay.write_melody(&empty)?;

    relay.disconnect()?;
    println!("Melody cleared!");

    Ok(())
}

/// Check if data looks like RTTTL format (text with colons).
fn is_rtttl_format(data: &[u8]) -> bool {
    // RTTTL is ASCII text with format: name:settings:notes
    // Check if it's valid UTF-8 and contains the RTTTL structure
    if let Ok(text) = std::str::from_utf8(data) {
        let text = text.trim();
        // Must have at least 2 colons and be mostly printable ASCII
        let colon_count = text.chars().filter(|&c| c == ':').count();
        let is_printable = text
            .chars()
            .all(|c| c.is_ascii_graphic() || c.is_ascii_whitespace());
        colon_count >= 2 && is_printable && text.len() > 5
    } else {
        false
    }
}

/// Show melody information from file (binary or RTTTL).
pub fn melody_info(file: &str) -> Result<()> {
    if !Path::new(file).exists() {
        bail!("melody file not found: {file}");
    }

    let data = fs::read(file).with_context(|| format!("failed to read {file}"))?;

    println!("Melody file: {file}");
    println!("Size: {} bytes", data.len());

    if data.is_empty() {
        println!("(empty file)");
        return Ok(());
    }

    // Check if it's RTTTL format
    if is_rtttl_format(&data) {
        let text = std::str::from_utf8(&data).unwrap().trim();
        println!("Format: RTTTL");
        if let Some(name) = esc_protocol::Melody::rtttl_name(text) {
            println!("Name: {name}");
        }
        println!();

        match esc_protocol::Melody::from_rtttl(text) {
            Ok(melody) => {
                println!("Notes: {}", melody.note_count());
                println!("Entries: {} (count, freq_index) pairs", melody.len());
                println!();
                println!("Melody: {melody}");
                println!();
                print_melody_details(&melody);
            }
            Err(e) => {
                println!("Failed to parse RTTTL: {:?}", e);
                println!("Raw: {}", &text[..text.len().min(100)]);
            }
        }
        return Ok(());
    }

    // Binary format
    println!("Format: Binary (Bluejay)");

    if data[0] == esc_protocol::MELODY_EMPTY {
        println!("Status: No melody (first byte is 0xFF)");
        return Ok(());
    }

    if let Some(melody) = esc_protocol::Melody::from_bytes(&data) {
        println!("BPM: {}", melody.bpm());
        println!("Notes: {}", melody.note_count());
        println!("Entries: {} (count, freq_index) pairs", melody.len());
        println!();
        println!("Melody: {melody}");
        println!();
        print_melody_details(&melody);
    } else {
        println!("Failed to parse melody data");
        println!("Raw bytes: {:02X?}", &data[..data.len().min(32)]);
    }

    Ok(())
}

/// Print detailed entry breakdown.
fn print_melody_details(melody: &esc_protocol::Melody) {
    println!("Entry details:");
    for (i, entry) in melody.entries().iter().enumerate() {
        if entry.is_pause() {
            println!("  [{:2}] PAUSE  {}ms", i, entry.count);
        } else {
            println!(
                "  [{:2}] count={:3}  freq_idx={:3}  (~{:.1} Hz)",
                i,
                entry.count,
                entry.freq_index,
                entry.frequency()
            );
        }
    }
}

/// Convert RTTTL file to binary melody file.
pub fn melody_convert(input: &str, output: &str) -> Result<()> {
    if !Path::new(input).exists() {
        bail!("input file not found: {input}");
    }

    let data = fs::read(input).with_context(|| format!("failed to read {input}"))?;

    let text = std::str::from_utf8(&data)
        .with_context(|| "input file is not valid UTF-8 text")?
        .trim();

    let melody = esc_protocol::Melody::from_rtttl(text)
        .map_err(|e| anyhow::anyhow!("failed to parse RTTTL: {:?}", e))?;

    if let Some(name) = esc_protocol::Melody::rtttl_name(text) {
        println!("Converting: {name}");
    }
    println!("Notes: {}", melody.note_count());

    let bytes = melody.to_bytes();
    fs::write(output, bytes).with_context(|| format!("failed to write {output}"))?;

    println!("Wrote {} bytes to {output}", esc_protocol::MELODY_SIZE);
    Ok(())
}
