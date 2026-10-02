//! BLHeli/AM32 ESC protocol implementation.

#![no_std]

#[cfg(any(test, feature = "std"))]
extern crate std;

#[cfg(feature = "std")]
pub mod am32_config;
#[cfg(feature = "serde")]
pub use am32_config::Am32ConfigData;
#[cfg(feature = "std")]
pub use am32_config::{
    Am32Config, BootStatus, BrakeMode, MotorTiming, Setting, SignalType, VariablePwmMode,
    apply_setting, parse_bool,
};

pub mod melody;
pub use melody::{
    MELODY_EMPTY, MELODY_MAX_NOTES, MELODY_OFFSET, MELODY_SIZE, Melody, MelodyEntry, RtttlError,
};

pub mod bootloader;
pub mod config;
pub mod crc;
pub mod device;
pub mod ihex;
pub mod packet;

pub use crc::{crc16_ibm, crc16_ibm_bytes};
#[cfg(feature = "std")]
pub use ihex::{Firmware, LoadError, Segment, load_firmware};
pub use ihex::{ParseError, Record, RecordData, RecordType, parse_record};
pub use packet::*;
