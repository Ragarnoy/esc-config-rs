mod commands;
mod relay;
mod ui;

use clap::Parser;

#[derive(Parser)]
#[command(name = "esc-configurator")]
#[command(about = "CLI tool for configuring BLHeli_S and AM32 ESCs")]
#[command(version)]
struct Cli {
    /// Serial port device (shortcut for 'config -d')
    #[arg(short, long, global = true)]
    device: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Interactive configuration mode (default when no subcommand given)
    #[command(alias = "configure")]
    Config {
        /// Serial port device (will prompt if not provided)
        #[arg(short, long)]
        device: Option<String>,
    },

    /// Flash firmware to ESC
    Flash {
        /// Path to hex file
        #[arg(short, long)]
        file: String,

        /// Serial port device
        #[arg(short, long)]
        device: String,
    },

    /// Read configuration from ESC (raw hex dump)
    Read {
        /// Serial port device
        #[arg(short, long)]
        device: String,

        /// Output file to save config (optional)
        #[arg(short, long)]
        output: Option<String>,
    },

    /// Show parsed ESC configuration (human-readable)
    Show {
        /// Serial port device
        #[arg(short, long)]
        device: String,
    },

    /// Set a configuration value directly (for scripting)
    Set {
        /// Serial port device
        #[arg(short, long)]
        device: String,

        /// Setting name (e.g., motor-kv, timing, direction)
        #[arg(short, long)]
        setting: String,

        /// Value to set (e.g., 2200, on, off, 15)
        #[arg(short, long)]
        value: String,
    },

    /// List available settings
    Settings,

    /// Verify firmware on ESC matches hex file
    Verify {
        /// Path to hex file
        #[arg(short, long)]
        file: String,

        /// Serial port device
        #[arg(short, long)]
        device: String,
    },

    /// Backup ESC configuration to file
    Backup {
        /// Serial port device
        #[arg(short, long)]
        device: String,

        /// Output file path
        #[arg(short, long)]
        output: String,
    },

    /// Restore ESC configuration from backup file
    Restore {
        /// Serial port device
        #[arg(short, long)]
        device: String,

        /// Input backup file path
        #[arg(short, long)]
        input: String,
    },

    /// List available serial ports
    Ports,

    /// Display information about Intel HEX file
    HexInfo {
        /// Path to hex file
        #[arg(short, long)]
        file: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Determine which command to run (default to Config)
    let command = cli.command.unwrap_or(Commands::Config { device: None });

    match command {
        Commands::Config { device } => {
            // Use device from subcommand or fall back to global -d flag
            let device = device.or(cli.device);
            commands::interactive_config(device.as_deref())?;
        }
        Commands::Flash { file, device } => {
            commands::flash_firmware(&device, &file)?;
        }
        Commands::Read { device, output } => {
            commands::read_config(&device, output.as_deref())?;
        }
        Commands::Show { device } => {
            commands::show_config(&device)?;
        }
        Commands::Set {
            device,
            setting,
            value,
        } => {
            commands::set_config(&device, &setting, &value)?;
        }
        Commands::Settings => {
            commands::list_settings();
        }
        Commands::Verify { file, device } => {
            commands::verify_firmware(&device, &file)?;
        }
        Commands::Backup { device, output } => {
            commands::backup_config(&device, &output)?;
        }
        Commands::Restore { device, input } => {
            if !ui::confirm_operation("This will overwrite ESC configuration. Continue?")? {
                println!("Aborted.");
                return Ok(());
            }
            commands::restore_config(&device, &input)?;
        }
        Commands::Ports => {
            commands::list_ports()?;
        }
        Commands::HexInfo { file } => {
            commands::hex_info(&file)?;
        }
    }

    Ok(())
}
