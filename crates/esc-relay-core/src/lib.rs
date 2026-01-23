// Allow async fn in trait - we use Embassy's single-threaded executor, no Send bounds needed
#![allow(async_fn_in_trait)]
#![no_std]

//! Generic relay logic for ESC configurator probe.
//!
//! This crate provides the core logic for the relay MCU that bridges
//! between the PC (via USB serial) and the ESC (via DShot/UART).
//!
//! The relay operates in two modes:
//! 1. **DShot mode**: Send DShot commands to enter ESC bootloader
//! 2. **Passthrough mode**: Bridge USB serial to ESC UART at 19200 baud
//!
//! # Architecture
//!
//! ```text
//! PC (USB Serial) <-> Relay MCU <-> ESC
//!                      |
//!                      +-- DShot out (bootloader entry)
//!                      +-- UART (BLHeli protocol passthrough)
//! ```

#[cfg(feature = "std")]
extern crate std;

pub mod protocol;
pub mod state;
pub mod traits;

pub use protocol::{Command, Response};
pub use state::{Relay, RelayState};
pub use traits::{DelayNs, DshotOutput, DshotOutputAsync, Read, Write};
