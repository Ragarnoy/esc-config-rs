//! BLHeli/AM32 ESC protocol implementation.

#![no_std]

#[cfg(any(test, feature = "std"))]
extern crate std;

#[cfg(feature = "std")]
pub mod am32_config;
pub mod bootloader;
pub mod config;
pub mod crc;
pub mod device;
pub mod ihex;
pub mod packet;

pub use crc::{crc16_ibm, crc16_ibm_bytes};
pub use ihex::{parse_record, ParseError, Record, RecordData, RecordType};
#[cfg(feature = "std")]
pub use ihex::{load_firmware, Firmware, LoadError, Segment};
pub use packet::*;
