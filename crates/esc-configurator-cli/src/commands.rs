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
use esc_protocol::am32_config::{Am32Config, Setting, apply_setting};
use esc_protocol::config::ConfigBlock;
use esc_protocol::device::{DeviceInfo, FirmwareType};
use esc_protocol::ihex::{load_firmware, Firmware};

use crate::relay::Relay;
use crate::ui::{
    self, MainMenuAction, confirm_exit_with_unsaved, confirm_restore, edit_bool_setting,
    edit_direction_setting, edit_motor_kv_setting, edit_timing_setting, edit_u8_setting,
    get_setting_display_value, prompt_backup_path, prompt_restore_path, select_category,
    select_setting_in_category, show_main_menu,
};

/// Format device info for display.
fn format_device(info: &DeviceInfo) -> String {
    let firmware_name = match info.firmware {
        Some(FirmwareType::BlheliS) => "BLHeli_S",
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
    println!("Connected: {}", format_device(device_info));

    // Check if this is AM32
    if device_info.firmware != Some(FirmwareType::Am32) {
        relay.disconnect()?;
        bail!("Interactive config only supported for AM32 ESCs.");
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

    let esc_name = config.name().to_string();
    let mut has_unsaved_changes = false;

    // Main menu loop
    loop {
        match show_main_menu(&esc_name, has_unsaved_changes)? {
            MainMenuAction::ViewSettings => {
                println!("\n{}", config);
            }
            MainMenuAction::EditSettings => {
                if let Some(true) = edit_settings_loop(&mut config)? {
                    has_unsaved_changes = config.as_bytes() != original_config.as_bytes();
                }
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
        let category = match select_category()? {
            Some(c) => c,
            None => return Ok(Some(modified)),
        };

        // Select setting in category
        loop {
            let (setting, name) = match select_setting_in_category(category, config)? {
                Some(s) => s,
                None => break, // Back to category selection
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
        Setting::DirectionReversed => {
            if let Some(value) = edit_direction_setting(config.direction_reversed())? {
                config.set_direction_reversed(value);
                println!("Set {} = {}", name, get_setting_display_value(setting, config));
                return Ok(true);
            }
        }
        Setting::Bidirectional => {
            if let Some(value) = edit_bool_setting(name, config.bidirectional())? {
                config.set_bidirectional(value);
                println!("Set {} = {}", name, get_setting_display_value(setting, config));
                return Ok(true);
            }
        }
        Setting::SinusoidalStartup => {
            if let Some(value) = edit_bool_setting(name, config.sinusoidal_startup())? {
                config.set_sinusoidal_startup(value);
                println!("Set {} = {}", name, get_setting_display_value(setting, config));
                return Ok(true);
            }
        }
        Setting::ComplementaryPwm => {
            if let Some(value) = edit_bool_setting(name, config.complementary_pwm())? {
                config.set_complementary_pwm(value);
                println!("Set {} = {}", name, get_setting_display_value(setting, config));
                return Ok(true);
            }
        }
        Setting::VariablePwm => {
            if let Some(value) = edit_bool_setting(name, config.variable_pwm())? {
                config.set_variable_pwm(value);
                println!("Set {} = {}", name, get_setting_display_value(setting, config));
                return Ok(true);
            }
        }
        Setting::StuckRotorProtection => {
            if let Some(value) = edit_bool_setting(name, config.stuck_rotor_protection())? {
                config.set_stuck_rotor_protection(value);
                println!("Set {} = {}", name, get_setting_display_value(setting, config));
                return Ok(true);
            }
        }
        Setting::TimingAdvance => {
            if let Some(value) = edit_timing_setting(config.timing_advance())? {
                config.set_timing_advance(value);
                println!("Set {} = {}", name, get_setting_display_value(setting, config));
                return Ok(true);
            }
        }
        Setting::PwmFrequency => {
            if let Some(value) = edit_u8_setting(name, config.pwm_frequency_khz(), 16, 96)? {
                config.set_pwm_frequency_khz(value);
                println!("Set {} = {}", name, get_setting_display_value(setting, config));
                return Ok(true);
            }
        }
        Setting::StartupPower => {
            if let Some(value) = edit_u8_setting(name, config.startup_power(), 50, 150)? {
                config.set_startup_power(value);
                println!("Set {} = {}", name, get_setting_display_value(setting, config));
                return Ok(true);
            }
        }
        Setting::MotorKv => {
            if let Some(value) = edit_motor_kv_setting(config.motor_kv())? {
                config.set_motor_kv(value);
                println!("Set {} = {}", name, get_setting_display_value(setting, config));
                return Ok(true);
            }
        }
        Setting::MotorPoles => {
            if let Some(value) = edit_u8_setting(name, config.motor_poles(), 2, 36)? {
                config.set_motor_poles(value);
                println!("Set {} = {}", name, get_setting_display_value(setting, config));
                return Ok(true);
            }
        }
        Setting::BrakeOnStop => {
            if let Some(value) = edit_bool_setting(name, config.brake_on_stop())? {
                config.set_brake_on_stop(value);
                println!("Set {} = {}", name, get_setting_display_value(setting, config));
                return Ok(true);
            }
        }
        Setting::StallProtection => {
            if let Some(value) = edit_bool_setting(name, config.stall_protection())? {
                config.set_stall_protection(value);
                println!("Set {} = {}", name, get_setting_display_value(setting, config));
                return Ok(true);
            }
        }
        Setting::BeepVolume => {
            if let Some(value) = edit_u8_setting(name, config.beep_volume(), 0, 11)? {
                config.set_beep_volume(value);
                println!("Set {} = {}", name, get_setting_display_value(setting, config));
                return Ok(true);
            }
        }
        Setting::Telemetry30ms => {
            if let Some(value) = edit_bool_setting(name, config.telemetry_30ms())? {
                config.set_telemetry_30ms(value);
                println!("Set {} = {}", name, get_setting_display_value(setting, config));
                return Ok(true);
            }
        }
        Setting::ServoLow => {
            if let Some(value) = edit_u8_setting(name, config.servo_low(), 0, 255)? {
                config.set_servo_low(value);
                println!("Set {} = {}", name, get_setting_display_value(setting, config));
                return Ok(true);
            }
        }
        Setting::ServoHigh => {
            if let Some(value) = edit_u8_setting(name, config.servo_high(), 0, 255)? {
                config.set_servo_high(value);
                println!("Set {} = {}", name, get_setting_display_value(setting, config));
                return Ok(true);
            }
        }
        Setting::ServoNeutral => {
            if let Some(value) = edit_u8_setting(name, config.servo_neutral(), 0, 255)? {
                config.set_servo_neutral(value);
                println!("Set {} = {}", name, get_setting_display_value(setting, config));
                return Ok(true);
            }
        }
        Setting::LowVoltageCutoff => {
            if let Some(value) = edit_bool_setting(name, config.low_voltage_cutoff())? {
                config.set_low_voltage_cutoff(value);
                println!("Set {} = {}", name, get_setting_display_value(setting, config));
                return Ok(true);
            }
        }
        Setting::LvcThreshold => {
            if let Some(value) = edit_u8_setting(name, config.lvc_threshold(), 0, 255)? {
                config.set_lvc_threshold(value);
                println!("Set {} = {}", name, get_setting_display_value(setting, config));
                return Ok(true);
            }
        }
        Setting::RcCarReversing => {
            if let Some(value) = edit_bool_setting(name, config.rc_car_reversing())? {
                config.set_rc_car_reversing(value);
                println!("Set {} = {}", name, get_setting_display_value(setting, config));
                return Ok(true);
            }
        }
        Setting::UseHallSensors => {
            if let Some(value) = edit_bool_setting(name, config.use_hall_sensors())? {
                config.set_use_hall_sensors(value);
                println!("Set {} = {}", name, get_setting_display_value(setting, config));
                return Ok(true);
            }
        }
        Setting::SineModeRange => {
            if let Some(value) = edit_u8_setting(name, config.sine_mode_range(), 5, 25)? {
                config.set_sine_mode_range(value);
                println!("Set {} = {}", name, get_setting_display_value(setting, config));
                return Ok(true);
            }
        }
        Setting::DragBrakeStrength => {
            if let Some(value) = edit_u8_setting(name, config.drag_brake_strength(), 1, 10)? {
                config.set_drag_brake_strength(value);
                println!("Set {} = {}", name, get_setting_display_value(setting, config));
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
    println!("Connected: {}", format_device(device_info));

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
pub fn flash_firmware(device: &str, hex_file: &str) -> Result<()> {
    // Validate file exists
    if !Path::new(hex_file).exists() {
        bail!("hex file not found: {hex_file}");
    }

    println!("Connecting to ESC via {device}...");
    let mut relay = Relay::open(device)?;
    relay.connect()?;

    let device_info = relay.session().device().expect("should be connected");
    println!("Connected: {}", format_device(device_info));

    // TODO: Parse Intel HEX file
    // TODO: Flash each block
    // TODO: Verify after write

    println!("\nFlash not yet implemented - hex parsing needed");
    println!("File: {hex_file}");

    relay.disconnect()?;
    println!("Disconnected.");

    Ok(())
}

/// Verify ESC firmware against hex file.
pub fn verify_firmware(device: &str, hex_file: &str) -> Result<()> {
    if !Path::new(hex_file).exists() {
        bail!("hex file not found: {hex_file}");
    }

    println!("Connecting to ESC via {device}...");
    let mut relay = Relay::open(device)?;
    relay.connect()?;

    let device_info = relay.session().device().expect("should be connected");
    println!("Connected: {}", format_device(device_info));

    // TODO: Parse Intel HEX file
    // TODO: Read each block and compare

    println!("\nVerify not yet implemented - hex parsing needed");
    println!("File: {hex_file}");

    relay.disconnect()?;
    println!("Disconnected.");

    Ok(())
}

/// Backup ESC configuration to file.
pub fn backup_config(device: &str, output: &str) -> Result<()> {
    println!("Connecting to ESC via {device}...");
    let mut relay = Relay::open(device)?;
    relay.connect()?;

    let device_info = relay.session().device().expect("should be connected");
    println!("Connected: {}", format_device(device_info));

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
        bail!("invalid backup file: expected 256 bytes, got {}", data.len());
    }

    let config = ConfigBlock::from_bytes(&data);

    println!("Connecting to ESC via {device}...");
    let mut relay = Relay::open(device)?;
    relay.connect()?;

    let device_info = relay.session().device().expect("should be connected");
    println!("Connected: {}", format_device(device_info));

    println!("Writing configuration...");
    relay.write_config(&config)?;

    println!("Configuration restored from: {input}");

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

    let content = fs::read_to_string(hex_file)
        .with_context(|| format!("failed to read {hex_file}"))?;

    let firmware = load_firmware(&content)
        .with_context(|| format!("failed to parse {hex_file}"))?;

    println!("Intel HEX file: {hex_file}");
    println!();
    print_firmware_info(&firmware);

    Ok(())
}

/// Print detailed firmware information.
fn print_firmware_info(firmware: &Firmware) {
    println!("Segments: {}", firmware.segments.len());
    println!("Total size: {} bytes ({:.1} KB)",
        firmware.total_size(),
        firmware.total_size() as f64 / 1024.0
    );

    if let Some(min) = firmware.min_address() {
        println!("Address range: 0x{:08X} - 0x{:08X}",
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
        println!("  [{i}] 0x{:08X} - 0x{:08X} ({} bytes)",
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
    println!("Connected: {}", format_device(device_info));

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
    println!("Connected: {}", format_device(device_info));

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
