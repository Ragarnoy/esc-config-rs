//! DShot protocol implementation.
//!
//! DShot is a digital protocol for ESC communication used in drones.
//! This crate provides frame encoding/decoding, CRC calculation, and timing constants.
//!
//! # Example
//! ```
//! use esc_dshot::{encode, decode, Speed};
//!
//! // Encode a throttle command
//! let frame = encode(1000, true); // throttle=1000, telemetry=true
//!
//! // Decode a received frame
//! let decoded = decode(frame);
//! assert_eq!(decoded.throttle, 1000);
//! assert!(decoded.telemetry);
//! assert!(decoded.crc_valid);
//! ```

#![no_std]

#[cfg(any(test, feature = "std"))]
extern crate std;

pub mod crc4;
pub mod frame;
pub mod timing;

// Re-export commonly used items
pub use crc4::{crc4, verify_crc4};
pub use frame::{decode, encode, encode_command, encode_throttle, Command, DecodedFrame};
pub use frame::{COMMAND_COUNT, FRAME_BITS, THROTTLE_MAX, THROTTLE_MIN};
pub use timing::Speed;
