//! Typed operations for configuring an AM32 ESC, as postcard-rpc endpoints.
//!
//! A server (the relay firmware, or Elle) runs `esc-session` against the ESC and
//! answers these requests; each is one whole operation, so no bootloader sequence
//! ever crosses the transport. Every response is a `Result<_, EscError>`.
//!
//! The relay serves the endpoints below at `esc/…`. Elle reuses the request and
//! response types in its own `elle/esc/…` endpoints.

#![cfg_attr(not(feature = "use-std"), no_std)]

use core::fmt;

use esc_protocol::layout::Layout;
use heapless::Vec;
use postcard_rpc::{TopicDirection, endpoints, topics};
use postcard_schema::Schema;
use serde::{Deserialize, Serialize};

/// Bytes of the AM32 settings structure (config, melody, CAN, reserved).
pub const EEPROM_SIZE: usize = 192;
/// Largest read or write in one request.
pub const MAX_BLOCK: usize = esc_protocol::packet::MAX_BLOCK;

/// USB VID:PID of the relay firmware (pid.codes test PID).
pub const RELAY_USB_VID_PID: (u16, u16) = (0x1209, 0x0001);

/// The settings structure, exactly [`EEPROM_SIZE`] bytes.
pub type Eeprom = Vec<u8, EEPROM_SIZE>;
/// Up to [`MAX_BLOCK`] bytes.
pub type Block = Vec<u8, MAX_BLOCK>;

// ============================================================================
// Requests
// ============================================================================

/// Raw read at a protocol (Set Address) address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct ReadReq {
    /// Protocol address (shifted on 128 KiB parts).
    pub address: u16,
    /// Bytes, 1..=256.
    pub len: u16,
}

/// One chunk of a firmware image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct ChunkReq {
    /// Byte offset from the app start; a multiple of 8.
    pub offset: u32,
    /// 1..=256 bytes. Padded with 0xFF to a multiple of 8 by the server.
    pub data: Block,
}

/// Read part of the app region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct ReadFirmwareReq {
    /// Byte offset from the app start.
    pub offset: u32,
    /// Bytes, 1..=256.
    pub len: u16,
}

// ============================================================================
// Responses
// ============================================================================

/// Flash layout in protocol units (see `esc_protocol::layout::Layout`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct LayoutInfo {
    /// Left shift the bootloader applies to addresses (0, or 2 on 128 KiB parts).
    pub address_shift: u8,
    /// App start.
    pub firmware_start: u16,
    /// 32-byte firmware name.
    pub filename_start: u16,
    /// EEPROM base.
    pub eeprom_start: u16,
}

impl From<Layout> for LayoutInfo {
    fn from(l: Layout) -> Self {
        Self {
            address_shift: l.address_shift,
            firmware_start: l.firmware_start,
            filename_start: l.filename_start,
            eeprom_start: l.eeprom_start,
        }
    }
}

impl From<LayoutInfo> for Layout {
    fn from(l: LayoutInfo) -> Self {
        Self {
            address_shift: l.address_shift,
            firmware_start: l.firmware_start,
            filename_start: l.filename_start,
            eeprom_start: l.eeprom_start,
        }
    }
}

/// A connected ESC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct EscInfo {
    /// `PIN | FLASH << 8` from the connect reply.
    pub device_id: u16,
    /// Signal pin code (`port << 4 | pin`).
    pub pin_code: u8,
    /// Flash size code (`0x1F` 32K, `0x35` 64K, `0x2B` 128K).
    pub flash_code: u8,
    /// Bootloader protocol version.
    pub protocol_version: u8,
    /// Flash layout.
    pub layout: LayoutInfo,
    /// Bytes available for the app image.
    pub max_firmware_size: u32,
}

/// The server (relay firmware) identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct ServerInfo {
    /// Firmware version, major.
    pub major: u8,
    /// Firmware version, minor.
    pub minor: u8,
    /// Firmware version, patch.
    pub patch: u8,
}

/// What kind of unsupported device answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub enum DeviceKind {
    /// SiLabs bootloader (BLHeli_S).
    BlheliS,
    /// ATmega bootloader.
    Atmega,
    /// Anything else.
    Unknown,
}

/// Why an operation failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub enum EscError {
    /// No ESC connected: send `connect` first.
    NotConnected,
    /// The server's link to the ESC failed.
    Link,
    /// The ESC did not answer.
    NoReply,
    /// The ESC answered with fewer bytes than expected.
    ShortReply {
        /// Bytes expected.
        expected: u16,
        /// Bytes received.
        got: u16,
    },
    /// The ESC answered with something other than ACK (`0xC1` refused, `0xC2` CRC).
    Nack(u8),
    /// The connect reply was not an AM32 bootloader's.
    BadConnectReply,
    /// Not an AM32 ESC.
    Unsupported {
        /// Device id from the connect reply.
        device_id: u16,
        /// What it looks like.
        kind: DeviceKind,
    },
    /// AM32, but the flash layout is unknown.
    UnknownLayout {
        /// Device id from the connect reply.
        device_id: u16,
    },
    /// Read data failed its CRC repeatedly.
    ReadCrc,
    /// A length the bootloader cannot take.
    InvalidLength(u16),
    /// Address not representable for this ESC.
    BadAddress,
    /// Image or chunk beyond the app region.
    ImageTooLarge {
        /// End of the data in bytes from the app start.
        size: u32,
        /// App region size in bytes.
        max: u32,
    },
    /// Flash contents differ at this app offset.
    Verify {
        /// Byte offset from the app start.
        offset: u32,
    },
    /// Flash requests out of order: a chunk without `flash/begin`, `flash/finish`
    /// after a failed chunk, or an `eeprom/write` setting the boot byte while a
    /// flash is unfinished (the boot byte stays cleared).
    FlashSequence,
}

impl fmt::Display for EscError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConnected => write!(f, "no ESC connected"),
            Self::Link => write!(f, "the relay's link to the ESC failed"),
            Self::NoReply => write!(f, "no reply from the ESC"),
            Self::ShortReply { expected, got } => {
                write!(f, "short reply from the ESC: {got} of {expected} bytes")
            }
            Self::Nack(0xC2) => write!(f, "the ESC reported a CRC error (0xC2)"),
            Self::Nack(b) => write!(f, "the ESC refused the command (0x{b:02X})"),
            Self::BadConnectReply => {
                write!(f, "unexpected connect reply (not an AM32 bootloader?)")
            }
            Self::Unsupported { device_id, kind } => {
                let kind = match kind {
                    DeviceKind::BlheliS => "BLHeli_S (SiLabs) ESC",
                    DeviceKind::Atmega => "ATmega ESC",
                    DeviceKind::Unknown => "device",
                };
                write!(
                    f,
                    "unsupported {kind}: 0x{device_id:04X} (only AM32 is supported)"
                )
            }
            Self::UnknownLayout { device_id } => write!(
                f,
                "AM32 ESC 0x{device_id:04X}: flash layout unknown (bootloader too old to report it)"
            ),
            Self::ReadCrc => write!(f, "read data failed its CRC check repeatedly"),
            Self::InvalidLength(n) => write!(f, "invalid transfer length: {n} bytes"),
            Self::BadAddress => write!(f, "address not representable for this ESC"),
            Self::ImageTooLarge { size, max } => {
                write!(f, "firmware data reaches {size} bytes, only {max} fit")
            }
            Self::Verify { offset } => {
                write!(f, "flash verify failed at app offset 0x{offset:05X}")
            }
            Self::FlashSequence => write!(
                f,
                "flash out of sequence: begin, then every chunk, then finish; the boot byte stays cleared until a flash finishes"
            ),
        }
    }
}

#[cfg(feature = "use-std")]
impl std::error::Error for EscError {}

/// `connect` response.
pub type ConnectResp = Result<EscInfo, EscError>;
/// Response carrying no data.
pub type AckResp = Result<(), EscError>;
/// `read_eeprom` response.
pub type EepromResp = Result<Eeprom, EscError>;
/// Response carrying up to 256 bytes.
pub type BlockResp = Result<Block, EscError>;

// ============================================================================
// Endpoints
// ============================================================================

endpoints! {
    list = ENDPOINT_LIST;
    | EndpointTy            | RequestTy       | ResponseTy  | Path                 |
    | ----------            | ---------       | ----------  | ----                 |
    | ServerInfoEndpoint    | ()              | ServerInfo  | "esc/server/info"    |
    | ConnectEndpoint       | ()              | ConnectResp | "esc/connect"        |
    | DisconnectEndpoint    | ()              | AckResp     | "esc/disconnect"     |
    | RunEndpoint           | ()              | AckResp     | "esc/run"            |
    | ReadEepromEndpoint    | ()              | EepromResp  | "esc/eeprom/read"    |
    | WriteEepromEndpoint   | Eeprom          | AckResp     | "esc/eeprom/write"   |
    | ReadFileNameEndpoint  | ()              | BlockResp   | "esc/name"           |
    | ReadEndpoint          | ReadReq         | BlockResp   | "esc/read"           |
    | FlashBeginEndpoint    | ()              | AckResp     | "esc/flash/begin"    |
    | FlashChunkEndpoint    | ChunkReq        | AckResp     | "esc/flash/chunk"    |
    | FlashFinishEndpoint   | ()              | AckResp     | "esc/flash/finish"   |
    | ReadFirmwareEndpoint  | ReadFirmwareReq | BlockResp   | "esc/flash/read"     |
}

topics! {
    list = TOPICS_IN_LIST;
    direction = TopicDirection::ToServer;
    | TopicTy | MessageTy | Path |
    | ------- | --------- | ---- |
}

topics! {
    list = TOPICS_OUT_LIST;
    direction = TopicDirection::ToClient;
    | TopicTy | MessageTy | Path |
    | ------- | --------- | ---- |
}

// ============================================================================
// Conversions from the session (servers)
// ============================================================================

#[cfg(feature = "session")]
mod session {
    use esc_protocol::device::BootloaderType;
    use esc_session::{Error, Esc, Link};

    use super::{DeviceKind, EscError, EscInfo};

    // The ICD restates this so it builds without the session; keep them in step.
    const _: () = assert!(super::EEPROM_SIZE == esc_session::EEPROM_SIZE);

    fn len16(n: usize) -> u16 {
        u16::try_from(n).unwrap_or(u16::MAX)
    }

    fn len32(n: usize) -> u32 {
        u32::try_from(n).unwrap_or(u32::MAX)
    }

    impl<E> From<Error<E>> for EscError {
        fn from(e: Error<E>) -> Self {
            match e {
                Error::Link(_) => Self::Link,
                Error::NoReply => Self::NoReply,
                Error::ShortReply { expected, got } => Self::ShortReply {
                    expected: len16(expected),
                    got: len16(got),
                },
                Error::Nack(b) => Self::Nack(b),
                Error::BadConnectReply => Self::BadConnectReply,
                Error::Unsupported(info) => Self::Unsupported {
                    device_id: info.device_id,
                    kind: match info.bootloader {
                        BootloaderType::Sil => DeviceKind::BlheliS,
                        BootloaderType::Atm => DeviceKind::Atmega,
                        BootloaderType::Arm | BootloaderType::Unknown => DeviceKind::Unknown,
                    },
                },
                Error::UnknownLayout(info) => Self::UnknownLayout {
                    device_id: info.device_id,
                },
                Error::ReadCrc => Self::ReadCrc,
                Error::InvalidLength(n) => Self::InvalidLength(len16(n)),
                Error::BadAddress => Self::BadAddress,
                Error::ImageTooLarge { size, max } => Self::ImageTooLarge {
                    size: len32(size),
                    max: len32(max),
                },
                Error::Verify { offset } => Self::Verify {
                    offset: len32(offset),
                },
            }
        }
    }

    impl EscInfo {
        /// Describe a connected session.
        pub fn of<L: Link>(esc: &Esc<L>) -> Self {
            let reply = esc.connect_reply();
            Self {
                device_id: reply.device_id,
                pin_code: reply.pin_code,
                flash_code: reply.flash_code,
                protocol_version: reply.protocol_version,
                layout: (*esc.layout()).into(),
                max_firmware_size: len32(esc.max_firmware_size()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use postcard::{from_bytes, to_slice};

    fn round_trip<T>(value: &T) -> usize
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + fmt::Debug,
    {
        let mut buf = [0u8; 1024];
        let used = to_slice(value, &mut buf).unwrap();
        let n = used.len();
        let back: T = from_bytes(&buf[..n]).unwrap();
        assert_eq!(&back, value);
        n
    }

    fn info() -> EscInfo {
        EscInfo {
            device_id: 0x2B06,
            pin_code: 0x06,
            flash_code: 0x2B,
            protocol_version: 3,
            layout: LayoutInfo {
                address_shift: 2,
                firmware_start: 0x0400,
                filename_start: 0x7DF8,
                eeprom_start: 0x7E00,
            },
            max_firmware_size: 0x1E800,
        }
    }

    #[test]
    fn requests_and_responses_round_trip() {
        round_trip(&ReadReq {
            address: 0x7C00,
            len: 256,
        });
        round_trip(&ReadFirmwareReq {
            offset: 0x1_0000,
            len: 256,
        });
        round_trip(&ConnectResp::Ok(info()));
        round_trip(&AckResp::Ok(()));
        round_trip(&AckResp::Err(EscError::Nack(0xC2)));
        round_trip(&EepromResp::Ok(
            Vec::from_slice(&[0x5A; EEPROM_SIZE]).unwrap(),
        ));
        round_trip(&BlockResp::Err(EscError::ImageTooLarge {
            size: 30_000,
            max: 27_648,
        }));
        round_trip(&ServerInfo {
            major: 0,
            minor: 1,
            patch: 0,
        });
    }

    #[test]
    fn largest_request_fits_the_frame_buffers() {
        let chunk = ChunkReq {
            offset: u32::MAX,
            data: Vec::from_slice(&[0xFF; MAX_BLOCK]).unwrap(),
        };
        // + postcard-rpc header (≤ 13 bytes) + COBS overhead (~1/254): well under
        // the 1024-byte frame buffers on both ends.
        assert!(round_trip(&chunk) < 300);
    }

    #[test]
    fn layout_converts_both_ways() {
        let layout = Layout::from(info().layout);
        assert_eq!(layout.firmware_byte_offset(), 0x1000);
        assert_eq!(LayoutInfo::from(layout), info().layout);
    }

    #[cfg(feature = "session")]
    #[test]
    fn session_errors_convert() {
        use esc_protocol::device::classify_device;
        use esc_session::Error;

        let e: EscError = Error::<()>::Unsupported(classify_device(0xF390)).into();
        assert_eq!(
            e,
            EscError::Unsupported {
                device_id: 0xF390,
                kind: DeviceKind::BlheliS
            }
        );
        let e: EscError = Error::<()>::Verify { offset: 512 }.into();
        assert_eq!(e, EscError::Verify { offset: 512 });
        let e: EscError = Error::Link("uart").into();
        assert_eq!(e, EscError::Link);
    }
}
