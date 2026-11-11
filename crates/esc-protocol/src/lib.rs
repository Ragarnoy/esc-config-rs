//! AM32 ESC bootloader protocol: frames, CRC, device classification, flash layout,
//! EEPROM settings layout, melodies and Intel HEX parsing. Pure logic, no I/O.

#![no_std]

#[cfg(any(test, feature = "std"))]
extern crate std;

pub mod am32_config;
#[cfg(feature = "serde")]
pub use am32_config::Am32ConfigData;
pub use am32_config::{
    Am32Config, BootStatus, BrakeMode, MotorTiming, Setting, SignalType, VariablePwmMode,
    apply_setting, parse_bool,
};

pub mod melody;
pub use melody::{
    MELODY_EMPTY, MELODY_MAX_NOTES, MELODY_OFFSET, MELODY_SIZE, Melody, MelodyEntry, RtttlError,
};

pub mod crc;
pub mod device;
pub mod ihex;
pub mod layout;
pub mod packet;
pub mod policy;

pub use crc::{crc16_ibm, crc16_ibm_bytes};
#[cfg(feature = "std")]
pub use ihex::{Firmware, LoadError, Segment, load_firmware};
pub use ihex::{ParseError, Record, RecordData, RecordType, parse_record};
pub use layout::Layout;
pub use packet::*;
