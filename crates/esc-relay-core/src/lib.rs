// Allow async fn in trait - we use Embassy's single-threaded executor, no Send bounds needed
#![allow(async_fn_in_trait)]
#![no_std]

//! Generic relay logic for ESC configurator probe.
//!
//! This crate provides the core logic for the relay MCU that bridges
//! between the PC (via USB serial) and the ESC (via UART).
//!
//! The relay operates as a transparent UART bridge at 19200 baud,
//! forwarding BLHeli/AM32 bootloader protocol between USB and ESC.
//! Write operations are handled atomically to avoid USB latency issues.

#[cfg(feature = "std")]
extern crate std;

pub mod protocol;
pub mod state;
pub mod traits;

pub use protocol::{Command, Response};
pub use state::{Relay, RelayState};
pub use traits::{DelayNs, Read, Write};
