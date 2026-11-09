//! Session errors.

use core::fmt;

use esc_protocol::device::{BootloaderType, DeviceInfo};

/// What went wrong talking to the bootloader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error<E> {
    /// The link failed.
    Link(E),
    /// No reply before the timeout.
    NoReply,
    /// Part of a reply arrived before the timeout.
    ShortReply {
        /// Bytes expected.
        expected: usize,
        /// Bytes received.
        got: usize,
    },
    /// The ESC answered with something other than ACK (`0xC1` refused, `0xC2` bad CRC).
    Nack(u8),
    /// The connect reply was not `"471" ... 0x30`.
    BadConnectReply,
    /// Not an AM32 ESC.
    Unsupported(DeviceInfo),
    /// AM32, but the flash layout is unknown (NXP part without devinfo).
    UnknownLayout(DeviceInfo),
    /// Read data still failed its CRC after the retries.
    ReadCrc,
    /// A length the bootloader cannot take (reads and writes: 1..=256; writes: a
    /// multiple of 8).
    InvalidLength(usize),
    /// The address is not a whole number of protocol units, or out of range.
    BadAddress,
    /// The firmware image does not fit between the app start and the file name.
    ImageTooLarge {
        /// Image size in bytes.
        size: usize,
        /// Space available in bytes.
        max: usize,
    },
    /// Flash contents differ from the image at this byte offset (from app start).
    Verify {
        /// Byte offset of the first differing chunk.
        offset: usize,
    },
    /// An EEPROM write would set the boot byte (byte 0) to `0x01` on an ESC whose
    /// boot byte is not `0x01`, e.g. after a failed flash. Only
    /// [`crate::Esc::finish_flash`] may do that.
    WouldEnableBoot,
}

impl<E> From<E> for Error<E> {
    fn from(e: E) -> Self {
        Self::Link(e)
    }
}

impl<E: fmt::Display> fmt::Display for Error<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Link(e) => write!(f, "link error: {e}"),
            Self::NoReply => write!(f, "no reply from the ESC"),
            Self::ShortReply { expected, got } => {
                write!(f, "short reply from the ESC: {got} of {expected} bytes")
            }
            Self::Nack(0xC2) => write!(f, "the ESC reported a CRC error (0xC2)"),
            Self::Nack(b) => write!(f, "the ESC refused the command (0x{b:02X})"),
            Self::BadConnectReply => {
                write!(f, "unexpected connect reply (not an AM32 bootloader?)")
            }
            Self::Unsupported(info) => {
                let kind = match info.bootloader {
                    BootloaderType::Sil => "BLHeli_S (SiLabs) ESC",
                    BootloaderType::Atm => "ATmega ESC",
                    BootloaderType::Arm | BootloaderType::Unknown => "device",
                };
                write!(
                    f,
                    "unsupported {kind}: 0x{:04X} (only AM32 is supported)",
                    info.device_id
                )
            }
            Self::UnknownLayout(info) => write!(
                f,
                "AM32 ESC 0x{:04X}: flash layout unknown (bootloader too old to report it)",
                info.device_id
            ),
            Self::ReadCrc => write!(f, "read data failed its CRC check repeatedly"),
            Self::InvalidLength(n) => write!(f, "invalid transfer length: {n} bytes"),
            Self::BadAddress => write!(f, "address not representable for this ESC"),
            Self::ImageTooLarge { size, max } => {
                write!(f, "firmware image is {size} bytes, only {max} fit")
            }
            Self::Verify { offset } => {
                write!(f, "flash verify failed at app offset 0x{offset:05X}")
            }
            Self::WouldEnableBoot => write!(
                f,
                "settings write would set the boot byte; only a finished flash does that"
            ),
        }
    }
}

impl<E: fmt::Debug + fmt::Display> core::error::Error for Error<E> {}
