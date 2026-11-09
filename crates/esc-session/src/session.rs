//! The bootloader session: every sequencing and timing rule lives here.

use esc_protocol::device::{DeviceInfo, classify_device};
use esc_protocol::layout::{DEVINFO_LEN, Layout};
use esc_protocol::packet::{
    ACK, ADDR_MAGIC_DEVINFO, BAD_CRC, CONNECT_RESPONSE_LEN, ConnectResponse, MAX_BLOCK,
    ReadReplyError, build_connect_packet, build_payload, build_prog_flash_packet,
    build_read_packet, build_run_packet, build_set_address_packet, build_set_buffer_packet,
    parse_connect_response, parse_read_reply,
};

use crate::error::Error;
use crate::link::{Duration, Link};

/// Bytes of the AM32 settings structure (`EEprom_t`): config 0–47, melody 48–175,
/// CAN 176–183, reserved 184–191. Always written whole: a write at the EEPROM base
/// erases the page.
pub const EEPROM_SIZE: usize = 192;

/// Bytes of the firmware file name.
pub const FILE_NAME_SIZE: usize = 32;

/// Writes must be a multiple of this many bytes (G071 programs 8-byte words).
pub const WRITE_ALIGN: usize = 8;

/// EEPROM byte 0: the bootloader starts the app only when it is `0x01`.
const BOOT_BYTE: usize = 0;

/// Timeouts and retry counts. The defaults are for a direct wire to the ESC; a link
/// with its own latency (USB to a relay) should add to them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// Wait for the 9-byte connect reply.
    pub connect_reply: Duration,
    /// Wait for the ACK to Set Address and to a payload.
    pub ack: Duration,
    /// Wait for the ACK to Prog Flash (page erase + program + verify on the ESC).
    pub prog_flash: Duration,
    /// Wait for a read reply: `read_base + read_per_byte × (len + 3)`.
    pub read_base: Duration,
    /// See `read_base`. One byte at 19200 8N1 takes 521 µs.
    pub read_per_byte: Duration,
    /// Idle time between Set Buffer and its payload, so the bootloader sees two
    /// frames. Must exceed ~230 µs.
    pub set_buffer_gap: Duration,
    /// Quiet time that ends a drain of stray reply bytes.
    pub drain: Duration,
    /// Connect attempts.
    pub connect_attempts: u8,
    /// Extra attempts for a read or a payload that failed.
    pub retries: u8,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            connect_reply: Duration::from_millis(100),
            ack: Duration::from_millis(20),
            prog_flash: Duration::from_millis(100),
            read_base: Duration::from_millis(50),
            read_per_byte: Duration::from_micros(600),
            set_buffer_gap: Duration::from_millis(1),
            drain: Duration::from_millis(5),
            connect_attempts: 3,
            retries: 3,
        }
    }
}

impl Timing {
    fn read(&self, len: usize) -> Duration {
        #[allow(clippy::cast_possible_truncation)] // len <= 256
        self.read_base
            .saturating_add(self.read_per_byte.saturating_mul(len as u32 + 3))
    }
}

/// A connected AM32 bootloader.
pub struct Esc<L: Link> {
    link: L,
    timing: Timing,
    reply: ConnectResponse,
    device: DeviceInfo,
    layout: Layout,
}

type Result<T, L> = core::result::Result<T, Error<<L as Link>::Error>>;

impl<L: Link> Esc<L> {
    /// Connect with the default [`Timing`].
    pub async fn connect(link: L) -> Result<Self, L> {
        Self::connect_with(link, Timing::default()).await
    }

    /// Send the connect handshake, classify the ESC and work out its flash layout.
    ///
    /// The layout comes from the bootloader's devinfo when it speaks protocol 3+
    /// (this is the only way to find a DroneCAN bootloader's app start), else from
    /// the flash size code. Non-AM32 ESCs are refused.
    pub async fn connect_with(mut link: L, timing: Timing) -> Result<Self, L> {
        let frame = build_connect_packet();
        let mut buf = [0u8; CONNECT_RESPONSE_LEN];
        let mut last = Error::NoReply;
        let mut reply = None;
        for _ in 0..timing.connect_attempts.max(1) {
            link.send(&frame).await?;
            let n = link.recv(&mut buf, timing.connect_reply).await?;
            if n == buf.len() {
                if let Some(r) = parse_connect_response(&buf) {
                    reply = Some(r);
                    break;
                }
                last = Error::BadConnectReply;
            } else if n > 0 {
                last = Error::ShortReply {
                    expected: buf.len(),
                    got: n,
                };
            }
            drain(&mut link, timing.drain).await?;
        }
        let reply = reply.ok_or(last)?;

        let device = classify_device(reply.device_id);
        if !device.is_supported() {
            return Err(Error::Unsupported(device));
        }

        let fallback = Layout::from_flash_code(reply.flash_code);
        let mut esc = Self {
            link,
            timing,
            reply,
            device,
            layout: fallback.unwrap_or(Layout {
                address_shift: 0,
                firmware_start: 0,
                filename_start: 0,
                eeprom_start: 0,
            }),
        };
        let mut layout = fallback;
        if reply.protocol_version >= 3 {
            let mut devinfo = [0u8; DEVINFO_LEN];
            if esc.read_at(ADDR_MAGIC_DEVINFO, &mut devinfo).await.is_ok() {
                layout = Layout::parse_devinfo(&devinfo).or(layout);
            }
        }
        esc.layout = layout.ok_or(Error::UnknownLayout(device))?;
        Ok(esc)
    }

    /// Rebuild a session from an earlier connect, without a new handshake.
    ///
    /// For a server that keeps its link between requests: it stores the connect
    /// reply and layout and resumes over `&mut link` for each request. The
    /// bootloader keeps no session state, so this is safe as long as the ESC has not
    /// been reset in between (a reset makes the next command fail, not misbehave).
    pub fn resume(link: L, timing: Timing, reply: ConnectResponse, layout: Layout) -> Self {
        Self {
            link,
            timing,
            reply,
            device: classify_device(reply.device_id),
            layout,
        }
    }

    /// The connect reply.
    pub const fn connect_reply(&self) -> &ConnectResponse {
        &self.reply
    }

    /// Device classification.
    pub const fn device(&self) -> &DeviceInfo {
        &self.device
    }

    /// Flash layout in protocol units.
    pub const fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Give the link back without leaving the bootloader.
    pub fn into_link(self) -> L {
        self.link
    }

    /// Leave the bootloader: send Run (`00 00 00 00`).
    ///
    /// The ESC starts its app if EEPROM byte 0 is `0x01` and the app looks valid;
    /// otherwise it stays in the bootloader. It replies `0xC1` either way.
    pub async fn run(mut self) -> Result<L, L> {
        self.link.send(&build_run_packet()).await?;
        let mut b = [0u8; 1];
        self.link.recv(&mut b, self.timing.ack).await?;
        Ok(self.link)
    }

    // ------------------------------------------------------------------------
    // Settings
    // ------------------------------------------------------------------------

    /// Read the 192-byte settings structure.
    pub async fn read_eeprom(&mut self) -> Result<[u8; EEPROM_SIZE], L> {
        let mut data = [0u8; EEPROM_SIZE];
        self.read_at(self.layout.eeprom_start, &mut data).await?;
        Ok(data)
    }

    /// Write the 192-byte settings structure.
    ///
    /// The whole structure goes in one write, because writing at the EEPROM base
    /// erases the page: config, melody and CAN settings must travel together. The
    /// bootloader replaces byte 2 with its own version. Byte 0 must be `0x01` for
    /// the ESC to boot its app.
    pub async fn write_eeprom(&mut self, data: &[u8; EEPROM_SIZE]) -> Result<(), L> {
        self.write_at(self.layout.eeprom_start, data).await
    }

    /// Read the 32-byte firmware file name (raw, NUL- or 0xFF-padded).
    pub async fn read_file_name(&mut self) -> Result<[u8; FILE_NAME_SIZE], L> {
        let mut name = [0u8; FILE_NAME_SIZE];
        self.read_at(self.layout.filename_start, &mut name).await?;
        Ok(name)
    }

    // ------------------------------------------------------------------------
    // Firmware
    // ------------------------------------------------------------------------

    /// Bytes available for the app: from its start to the EEPROM.
    pub const fn max_firmware_size(&self) -> usize {
        let units = self
            .layout
            .eeprom_start
            .saturating_sub(self.layout.firmware_start);
        (units as usize) << self.layout.address_shift
    }

    /// Read `buf.len()` bytes of the app region starting `offset` bytes in.
    pub async fn read_firmware(&mut self, offset: usize, buf: &mut [u8]) -> Result<(), L> {
        let mut done = 0;
        while done < buf.len() {
            let n = (buf.len() - done).min(MAX_BLOCK);
            let addr = self.firmware_address(offset + done)?;
            self.read_at(addr, &mut buf[done..done + n]).await?;
            done += n;
        }
        Ok(())
    }

    /// Flash an app image (bytes from the app start, as in a hex file's binary).
    ///
    /// [`Self::begin_flash`], then [`Self::write_firmware_chunk`] for each 256-byte
    /// chunk in order, then [`Self::finish_flash`]. On error, EEPROM byte 0 stays
    /// `0x00`, so the ESC stays in its bootloader instead of booting half an image.
    ///
    /// `progress(done, total)` is called after each chunk.
    pub async fn flash_firmware(
        &mut self,
        image: &[u8],
        mut progress: impl FnMut(usize, usize),
    ) -> Result<(), L> {
        self.check_image(image)?;
        self.begin_flash().await?;
        for (i, data) in image.chunks(MAX_BLOCK).enumerate() {
            let offset = i * MAX_BLOCK;
            self.write_firmware_chunk(offset, data).await?;
            progress(offset + data.len(), image.len());
        }
        self.finish_flash().await
    }

    /// Start a flash: clear EEPROM byte 0 so the bootloader will not start the app
    /// until [`Self::finish_flash`]. Idempotent.
    pub async fn begin_flash(&mut self) -> Result<(), L> {
        self.set_boot_byte(0x00).await
    }

    /// Write one chunk of the app image (1..=256 bytes) `offset` bytes from the app
    /// start, then read it back.
    ///
    /// `offset` must be a multiple of 8. The tail is padded with `0xFF` to a
    /// multiple of 8. Write chunks in increasing order: the first chunk at each page
    /// boundary erases the page.
    pub async fn write_firmware_chunk(&mut self, offset: usize, data: &[u8]) -> Result<(), L> {
        if data.is_empty() || data.len() > MAX_BLOCK {
            return Err(Error::InvalidLength(data.len()));
        }
        if !offset.is_multiple_of(WRITE_ALIGN) {
            return Err(Error::BadAddress);
        }
        let max = self.max_firmware_size();
        if offset + data.len() > max {
            return Err(Error::ImageTooLarge {
                size: offset + data.len(),
                max,
            });
        }

        let len = data.len().next_multiple_of(WRITE_ALIGN);
        let mut chunk = [0xFFu8; MAX_BLOCK];
        chunk[..data.len()].copy_from_slice(data);

        let addr = self.firmware_address(offset)?;
        self.write_at(addr, &chunk[..len]).await?;
        let mut readback = [0u8; MAX_BLOCK];
        self.read_at(addr, &mut readback[..len]).await?;
        if readback[..len] != chunk[..len] {
            return Err(Error::Verify { offset });
        }
        Ok(())
    }

    /// End a flash: set EEPROM byte 0 to `0x01` so the bootloader starts the app.
    pub async fn finish_flash(&mut self) -> Result<(), L> {
        self.set_boot_byte(0x01).await
    }

    async fn set_boot_byte(&mut self, value: u8) -> Result<(), L> {
        let mut eeprom = self.read_eeprom().await?;
        if eeprom[BOOT_BYTE] != value {
            eeprom[BOOT_BYTE] = value;
            self.write_eeprom(&eeprom).await?;
        }
        Ok(())
    }

    /// Compare the app region with an image. `progress(done, total)` per chunk.
    pub async fn verify_firmware(
        &mut self,
        image: &[u8],
        mut progress: impl FnMut(usize, usize),
    ) -> Result<(), L> {
        self.check_image(image)?;
        let mut buf = [0u8; MAX_BLOCK];
        for (i, data) in image.chunks(MAX_BLOCK).enumerate() {
            let offset = i * MAX_BLOCK;
            let addr = self.firmware_address(offset)?;
            self.read_at(addr, &mut buf[..data.len()]).await?;
            if buf[..data.len()] != *data {
                return Err(Error::Verify { offset });
            }
            progress(offset + data.len(), image.len());
        }
        Ok(())
    }

    fn check_image(&self, image: &[u8]) -> Result<(), L> {
        if image.is_empty() {
            return Err(Error::InvalidLength(0));
        }
        let max = self.max_firmware_size();
        if image.len() > max {
            return Err(Error::ImageTooLarge {
                size: image.len(),
                max,
            });
        }
        Ok(())
    }

    fn firmware_address(&self, offset: usize) -> Result<u16, L> {
        let bytes = u32::try_from(offset).map_err(|_| Error::BadAddress)?;
        self.layout
            .offset(self.layout.firmware_start, bytes)
            .ok_or(Error::BadAddress)
    }

    // ------------------------------------------------------------------------
    // Bootloader commands
    // ------------------------------------------------------------------------

    /// Set Address (protocol units). Retries if the ACK does not come.
    pub async fn set_address(&mut self, address: u16) -> Result<(), L> {
        let frame = build_set_address_packet(address);
        let mut last = Error::NoReply;
        for _ in 0..=self.timing.retries {
            self.link.send(&frame).await?;
            match self.expect_ack(self.timing.ack).await {
                Err(e @ (Error::NoReply | Error::ShortReply { .. })) => last = e,
                other => return other,
            }
        }
        Err(last)
    }

    /// Read `buf.len()` bytes (1..=256) at a protocol address, CRC-checked.
    ///
    /// The bootloader forgets the address after each read, so this sends Set
    /// Address every time. A short reply or a CRC mismatch is retried.
    pub async fn read_at(&mut self, address: u16, buf: &mut [u8]) -> Result<(), L> {
        let len = buf.len();
        #[allow(clippy::cast_possible_truncation)]
        let frame = build_read_packet(len as u16).ok_or(Error::InvalidLength(len))?;
        let timeout = self.timing.read(len);
        let mut reply = [0u8; MAX_BLOCK + 3];
        let mut last = Error::NoReply;

        for _ in 0..=self.timing.retries {
            self.set_address(address).await?;
            self.link.send(&frame).await?;
            let n = self.link.recv(&mut reply[..len + 3], timeout).await?;
            if n < len + 3 {
                last = if n == 0 {
                    Error::NoReply
                } else {
                    Error::ShortReply {
                        expected: len + 3,
                        got: n,
                    }
                };
                drain(&mut self.link, self.timing.drain).await?;
                continue;
            }
            match parse_read_reply(&reply[..n], len) {
                Ok(data) => {
                    buf.copy_from_slice(data);
                    return Ok(());
                }
                Err(ReadReplyError::BadCrc) => last = Error::ReadCrc,
                Err(ReadReplyError::NotAcked(b)) => return Err(Error::Nack(b)),
                Err(ReadReplyError::TooShort) => unreachable!("length checked above"),
            }
        }
        Err(last)
    }

    /// Write `data` (1..=256 bytes, a multiple of 8) at a protocol address.
    ///
    /// Set Address → Set Buffer → idle gap → payload + CRC → Prog Flash. The ESC
    /// programs and compares the flash before it ACKs Prog Flash.
    ///
    /// Recovery relies on a bootloader quirk: in payload mode it zero-fills a short
    /// frame and checks the CRC over the announced length, and a whole command
    /// frame followed by zeros has CRC 0, so any command frame is ACKed *as the
    /// payload* (`receiveBuffer`/`decodeInput`, AM32-bootloader `main.c`). A Set
    /// Address therefore always ends payload mode, either as itself or as a
    /// throwaway payload, and leaves the bootloader at a known address.
    ///
    /// - Payload answered `0xC2`: the bootloader is still in payload mode; resend.
    /// - Payload not answered: lost, or its ACK lost. Resending would run the
    ///   payload bytes as a command if the ACK was lost, so send Set Address and
    ///   Set Buffer again instead.
    /// - Giving up: send Set Address before returning, so the next operation's
    ///   first frame is not swallowed as a payload (its read or write would then
    ///   go to this address).
    pub async fn write_at(&mut self, address: u16, data: &[u8]) -> Result<(), L> {
        let len = data.len();
        if !len.is_multiple_of(WRITE_ALIGN) {
            return Err(Error::InvalidLength(len));
        }
        #[allow(clippy::cast_possible_truncation)]
        let set_buffer = build_set_buffer_packet(len as u16).ok_or(Error::InvalidLength(len))?;
        let mut payload = [0u8; MAX_BLOCK + 2];
        let n = build_payload(data, &mut payload).ok_or(Error::InvalidLength(len))?;

        self.set_address(address).await?;
        self.link.send(&set_buffer).await?;

        let mut last = Error::NoReply;
        let mut accepted = false;
        for _ in 0..=self.timing.retries {
            self.link.delay(self.timing.set_buffer_gap).await;
            self.link.send(&payload[..n]).await?;
            match self.expect_ack(self.timing.ack).await {
                Ok(()) => {
                    accepted = true;
                    break;
                }
                Err(e @ Error::Nack(BAD_CRC)) => last = e,
                Err(e @ Error::NoReply) => {
                    last = e;
                    self.set_address(address).await?;
                    self.link.send(&set_buffer).await?;
                }
                Err(e) => return Err(e),
            }
        }
        if !accepted {
            // Leave payload mode; the write failed either way.
            let _ = self.set_address(address).await;
            return Err(last);
        }

        // Prog Flash is idempotent here (same buffer, same address), so a lost ACK
        // can be retried.
        let prog = build_prog_flash_packet();
        let mut last = Error::NoReply;
        for _ in 0..=self.timing.retries {
            self.link.send(&prog).await?;
            match self.expect_ack(self.timing.prog_flash).await {
                Err(e @ Error::NoReply) => last = e,
                other => return other,
            }
        }
        Err(last)
    }

    async fn expect_ack(&mut self, timeout: Duration) -> Result<(), L> {
        let mut b = [0u8; 1];
        match self.link.recv(&mut b, timeout).await? {
            0 => Err(Error::NoReply),
            _ if b[0] == ACK => Ok(()),
            _ => Err(Error::Nack(b[0])),
        }
    }
}

/// Read and discard bytes until the line has been quiet for `quiet`.
async fn drain<L: Link>(link: &mut L, quiet: Duration) -> core::result::Result<(), L::Error> {
    let mut scratch = [0u8; 64];
    while link.recv(&mut scratch, quiet).await? > 0 {}
    Ok(())
}
