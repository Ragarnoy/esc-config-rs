//! AM32 bootloader session, `no_std` and async, over any half-duplex [`Link`].
//!
//! Owns the sequencing and timing rules of the bootloader protocol (see
//! `docs/REVAMP.md`, "How the bootloader actually behaves"): connect and layout
//! discovery, CRC-checked reads, Set Buffer → payload → Prog Flash writes with
//! recovery from a bad payload, the 192-byte settings block, and firmware flashing
//! with the boot byte as a safety latch. It has no hardware dependency; the relay
//! firmware, Elle and the CLI each provide a [`Link`].

#![no_std]

mod error;
mod link;
mod session;

pub use error::Error;
pub use link::{Duration, Link};
pub use session::{EEPROM_SIZE, Esc, FILE_NAME_SIZE, Timing, WRITE_ALIGN};
