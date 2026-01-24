//! BLHeli/AM32 ESC protocol implementation.

#![no_std]

#[cfg(any(test, feature = "std"))]
extern crate std;

#[cfg(feature = "std")]
pub mod am32_config;
#[cfg(feature = "std")]
pub use am32_config::{
    apply_setting, parse_bool, Am32Config, BootStatus, BrakeMode, MotorTiming, Setting,
    SignalType, VariablePwmMode,
};
#[cfg(feature = "serde")]
pub use am32_config::Am32ConfigData;

pub mod melody;
pub use melody::{
    Duration, Melody, MelodyNote, Note, RtttlError, MELODY_EMPTY, MELODY_MAX_NOTES, MELODY_OFFSET,
    MELODY_SIZE,
};

pub mod bootloader;
pub mod config;
pub mod crc;
pub mod device;
pub mod ihex;
pub mod packet;

// MSP and Four-Way protocol support
pub mod fourway;
pub mod fourway_session;
pub mod msp;

pub use crc::{crc16_ibm, crc16_ibm_bytes};
pub use ihex::{parse_record, ParseError, Record, RecordData, RecordType};
#[cfg(feature = "std")]
pub use ihex::{load_firmware, Firmware, LoadError, Segment};
pub use packet::*;

// MSP protocol re-exports
pub use msp::{MspCommand, MspError, MspResponse};

// Four-Way protocol re-exports
pub use fourway::{FourWayAck, FourWayCommand, FourWayError, FourWayMcuType, FourWayResponse};
pub use fourway_session::{FourWaySession, FourWaySessionError, FourWayState};
