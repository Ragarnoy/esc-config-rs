//! A simulated AM32 bootloader behind a [`Link`].
//!
//! Modelled on `am32-firmware/AM32-bootloader` (`bootloader/main.c` and
//! `Mcu/*/Src/eeprom.c`, rev 578ff29): one `send` is one frame; Set Buffer has no
//! reply and switches to payload mode, which a bad payload does not leave; Prog Flash
//! erases the page only at a page-aligned address, refuses misaligned writes and
//! programs by AND (so writing over unerased flash fails its compare); reads forget
//! the address; byte 2 of an EEPROM write becomes the bootloader version; Run
//! starts the app only if EEPROM byte 0 is 0x01.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::convert::Infallible;
use std::vec::Vec;

use esc_protocol::crc::{crc16_ibm, crc16_ibm_bytes};
use esc_protocol::layout::{DEVINFO_LEN, DEVINFO_MAGIC1, DEVINFO_MAGIC2};
use esc_session::{Duration, Link};

pub const BOOTLOADER_VERSION: u8 = 14;
const ACK: u8 = 0x30;
const BAD_ACK: u8 = 0xC1;
const BAD_CRC: u8 = 0xC2;
/// Idle time that ends a frame on the bootloader side (5 bit times from the middle
/// of the stop bit, ~234 µs; rounded up).
const FRAME_GAP: Duration = Duration::from_micros(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Address {
    Unset,
    Flash(usize),
    DevInfo,
}

/// MCU model for the fake.
#[derive(Debug, Clone, Copy)]
pub struct Mcu {
    pub flash_code: u8,
    pub flash_size: usize,
    pub page_size: usize,
    /// Address and length alignment for flash writes.
    pub write_align: usize,
    pub address_shift: u8,
    pub eeprom: usize,
    pub app_start: usize,
}

impl Mcu {
    pub const F051: Self = Self {
        flash_code: 0x1F,
        flash_size: 0x8000,
        page_size: 0x400,
        write_align: 2,
        address_shift: 0,
        eeprom: 0x7C00,
        app_start: 0x1000,
    };
    pub const F421: Self = Self {
        flash_code: 0x35,
        flash_size: 0x1_0000,
        page_size: 0x400,
        write_align: 4,
        address_shift: 0,
        eeprom: 0xF800,
        app_start: 0x1000,
    };
    pub const G071: Self = Self {
        flash_code: 0x2B,
        flash_size: 0x2_0000,
        page_size: 0x800,
        write_align: 8,
        address_shift: 2,
        eeprom: 0x1_F800,
        app_start: 0x1000,
    };
}

pub struct FakeEsc {
    pub mcu: Mcu,
    pub pin_code: u8,
    pub protocol_version: u8,
    /// File name address in bytes (DroneCAN builds put it inside the app).
    pub file_name_at: usize,
    pub flash: Vec<u8>,

    address: Address,
    continue_address: usize,
    payload_size: Option<usize>,
    buffer: Vec<u8>,
    replies: VecDeque<u8>,
    /// Idle time since the last frame ended.
    idle: Duration,
    last_was_set_buffer: bool,

    /// Frames as the bootloader saw them.
    pub frames: Vec<Vec<u8>>,
    /// The app was started by a Run.
    pub started_app: bool,
    /// The bootloader would jump to the app on its own (invalid_command > 100).
    pub invalid_commands: u32,

    // Fault injection.
    pub corrupt_payloads: u32,
    pub corrupt_reads: u32,
    pub drop_replies: u32,
    /// Drop the reply with this number (1-based, counting every reply).
    pub drop_reply_number: Option<u64>,
    replies_made: u64,
    /// Fail (0xC1) this many Prog Flash commands.
    pub fail_prog_flash: u32,
    /// Fail every Prog Flash whose byte address is in this range.
    pub fail_prog_in: Option<core::ops::Range<usize>>,
    /// Lose the next payload as if it ran into its Set Buffer frame.
    pub merge_next_payload: bool,
}

impl FakeEsc {
    pub fn new(mcu: Mcu) -> Self {
        let mut flash = vec![0xFFu8; mcu.flash_size];
        // A configured ESC: boot byte set, EEPROM version 4, firmware 2.19, some
        // settings, a melody and CAN settings.
        let e = mcu.eeprom;
        flash[e] = 0x01;
        flash[e + 1] = 4;
        flash[e + 2] = BOOTLOADER_VERSION;
        flash[e + 3] = 2;
        flash[e + 4] = 19;
        for i in 5..48 {
            flash[e + i] = i as u8;
        }
        for i in 48..176 {
            flash[e + i] = 0x80 | (i as u8 & 0x3F);
        }
        flash[e + 176..e + 184].copy_from_slice(&[7, 1, 1, 50, 0, 0, 0, 1]);
        let name = b"AM32_TEST_ESC";
        flash[e - 32..e - 32 + name.len()].copy_from_slice(name);
        flash[e - 32 + name.len()] = 0;
        // Some app so the vector table looks sane.
        flash[mcu.app_start..mcu.app_start + 8]
            .copy_from_slice(&[0, 0x10, 0, 0x20, 0xC1, 0x10, 0, 0x08]);

        Self {
            mcu,
            pin_code: 0x06,
            protocol_version: 3,
            file_name_at: e - 32,
            flash,
            address: Address::Unset,
            continue_address: 0,
            payload_size: None,
            buffer: Vec::new(),
            replies: VecDeque::new(),
            idle: Duration::from_millis(10),
            last_was_set_buffer: false,
            frames: Vec::new(),
            started_app: false,
            invalid_commands: 0,
            corrupt_payloads: 0,
            corrupt_reads: 0,
            drop_replies: 0,
            drop_reply_number: None,
            replies_made: 0,
            fail_prog_flash: 0,
            fail_prog_in: None,
            merge_next_payload: false,
        }
    }

    /// A DroneCAN bootloader: app at 0x4000, file name inside the app.
    pub fn dronecan(mcu: Mcu) -> Self {
        let mut esc = Self::new(Mcu {
            app_start: 0x4000,
            ..mcu
        });
        esc.file_name_at = 0x4000 + 0x200;
        esc
    }

    pub fn eeprom(&self) -> &[u8] {
        &self.flash[self.mcu.eeprom..self.mcu.eeprom + 192]
    }

    fn device_info(&self) -> [u8; 9] {
        [
            b'4',
            b'7',
            b'1',
            self.pin_code,
            self.mcu.flash_code,
            0x06,
            0x06,
            self.protocol_version,
            0x30,
        ]
    }

    fn devinfo(&self) -> [u8; DEVINFO_LEN] {
        let s = self.mcu.address_shift;
        let mut d = [0u8; DEVINFO_LEN];
        d[0..4].copy_from_slice(&DEVINFO_MAGIC1.to_le_bytes());
        d[4..8].copy_from_slice(&DEVINFO_MAGIC2.to_le_bytes());
        d[8..17].copy_from_slice(&self.device_info());
        d[17] = DEVINFO_LEN as u8;
        d[18] = s;
        let u = |bytes: usize| ((bytes >> s) as u16).to_le_bytes();
        d[19..21].copy_from_slice(&u(self.mcu.app_start));
        d[21..23].copy_from_slice(&u(self.file_name_at));
        d[23..25].copy_from_slice(&u(self.mcu.eeprom));
        d[25..27].copy_from_slice(&u(self.mcu.eeprom + 48));
        d
    }

    fn reply(&mut self, bytes: &[u8]) {
        self.replies_made += 1;
        if self.drop_reply_number == Some(self.replies_made) {
            return;
        }
        if self.drop_replies > 0 {
            self.drop_replies -= 1;
            return;
        }
        self.replies.extend(bytes);
    }

    fn bad(&mut self, byte: u8) {
        self.invalid_commands += 1;
        self.reply(&[byte]);
    }

    fn frame(&mut self, frame: &[u8]) {
        self.frames.push(frame.to_vec());

        if let Some(size) = self.payload_size {
            // receiveBuffer zero-fills rxBuffer and stops at size + 2 bytes;
            // decodeInput checks the CRC over `size` bytes, so a shorter frame that
            // ends in its own CRC (any command frame) passes: zeros keep CRC 0.
            let mut rx = [0u8; 258];
            let n = frame.len().min(size + 2);
            rx[..n].copy_from_slice(&frame[..n]);
            if self.corrupt_payloads > 0 {
                self.corrupt_payloads -= 1;
                rx[0] ^= 0x01;
            }
            let crc = u16::from_le_bytes([rx[size], rx[size + 1]]);
            if crc == crc16_ibm(&rx[..size]) {
                self.buffer = rx[..size].to_vec();
                self.payload_size = None;
                self.invalid_commands = 0;
                self.reply(&[ACK]);
            } else {
                self.bad(BAD_CRC); // still in payload mode
            }
            return;
        }

        let mut rx = [0u8; 258];
        let n = frame.len().min(rx.len());
        rx[..n].copy_from_slice(&frame[..n]);
        let crc_ok =
            |len: usize| u16::from_le_bytes([rx[len], rx[len + 1]]) == crc16_ibm(&rx[..len]);

        if rx[20] == 0x7D && rx[12] == 13 && rx[13] == 66 {
            let info = self.device_info();
            self.reply(&info);
            return;
        }

        match rx[0] {
            0x00 if rx[1] == 0 && rx[2] == 0 && rx[3] == 0 => {
                // CMD_RUN: jump (if allowed) after replying "bad command".
                self.reply(&[BAD_ACK]);
                if self.flash[self.mcu.eeprom] == 0x01 {
                    self.started_app = true;
                }
            }
            0x01 => {
                if !crc_ok(2) {
                    return self.bad(BAD_CRC);
                }
                let Address::Flash(addr) = self.address else {
                    return self.bad(BAD_ACK);
                };
                if addr < self.mcu.app_start {
                    return self.bad(BAD_ACK);
                }
                if self.fail_prog_flash > 0
                    || self
                        .fail_prog_in
                        .as_ref()
                        .is_some_and(|r| r.contains(&addr))
                {
                    self.fail_prog_flash = self.fail_prog_flash.saturating_sub(1);
                    return self.bad(BAD_ACK);
                }
                let mut data = self.buffer.clone();
                if addr == self.mcu.eeprom && data.len() > 2 {
                    data[2] = BOOTLOADER_VERSION;
                }
                if self.save_flash(addr, &data) {
                    self.invalid_commands = 0;
                    self.reply(&[ACK]);
                } else {
                    self.bad(BAD_ACK);
                }
            }
            0xFF => {
                if !crc_ok(4) {
                    return self.bad(BAD_CRC);
                }
                let a = u16::from_be_bytes([rx[2], rx[3]]) as usize;
                let s = self.mcu.address_shift;
                self.address = match a {
                    0x20 => Address::Flash(self.mcu.eeprom),
                    0x21 => Address::Flash(self.mcu.eeprom - 32),
                    0x22 => Address::Flash(self.continue_address),
                    0x23 => Address::DevInfo,
                    _ if a < 1024 => return self.bad(BAD_ACK),
                    _ => Address::Flash(a << s),
                };
                self.invalid_commands = 0;
                self.reply(&[ACK]);
            }
            0xFE => {
                if !crc_ok(4) {
                    return self.bad(BAD_CRC);
                }
                self.payload_size = Some(if rx[2] == 0x01 { 256 } else { rx[3] as usize });
                // no reply
            }
            0xFD => {
                if !crc_ok(2) {
                    return self.bad(BAD_CRC);
                }
                self.reply(&[BAD_ACK]);
            }
            0x02 => {
                if !crc_ok(2) {
                    return self.bad(BAD_CRC);
                }
                match self.address {
                    Address::Flash(a) if a >= self.mcu.app_start => self.reply(&[ACK]),
                    _ => self.bad(BAD_ACK),
                }
            }
            0x03 => {
                if !crc_ok(2) {
                    return self.bad(BAD_CRC);
                }
                let len = if rx[1] == 0 { 256 } else { rx[1] as usize };
                let mut data = match self.address {
                    Address::Unset => return self.bad(BAD_ACK),
                    Address::DevInfo => {
                        let mut d = vec![0xFFu8; len];
                        let info = self.devinfo();
                        let k = len.min(info.len());
                        d[..k].copy_from_slice(&info[..k]);
                        d
                    }
                    Address::Flash(a) => {
                        self.continue_address = a + len;
                        self.flash[a..a + len].to_vec()
                    }
                };
                let crc = crc16_ibm_bytes(&data);
                if self.corrupt_reads > 0 {
                    self.corrupt_reads -= 1;
                    data[0] ^= 0x01;
                }
                data.extend_from_slice(&crc);
                data.push(ACK);
                self.reply(&data);
                self.address = Address::Unset;
            }
            _ => self.bad(BAD_ACK),
        }
    }

    /// `save_flash_nolib`: alignment check, erase if page-aligned, program by AND,
    /// compare.
    fn save_flash(&mut self, addr: usize, data: &[u8]) -> bool {
        let align = self.mcu.write_align;
        if !addr.is_multiple_of(align) || !data.len().is_multiple_of(align) {
            return false;
        }
        if addr.is_multiple_of(self.mcu.page_size) {
            self.flash[addr..addr + self.mcu.page_size].fill(0xFF);
        }
        for (i, &b) in data.iter().enumerate() {
            self.flash[addr + i] &= b;
        }
        self.flash[addr..addr + data.len()] == *data
    }
}

impl Link for FakeEsc {
    type Error = Infallible;

    async fn send(&mut self, frame: &[u8]) -> Result<(), Infallible> {
        // A frame sent too soon after a Set Buffer runs into it: the bootloader
        // reads both as one frame and the payload bytes are lost.
        let merged = self.last_was_set_buffer
            && (self.idle < FRAME_GAP || core::mem::take(&mut self.merge_next_payload));
        self.last_was_set_buffer = false;
        if merged {
            self.frames.push(frame.to_vec());
        } else {
            let was_set_buffer = self.payload_size.is_none() && frame.first() == Some(&0xFE);
            self.frame(frame);
            self.last_was_set_buffer = was_set_buffer && self.payload_size.is_some();
        }
        self.idle = Duration::ZERO;
        Ok(())
    }

    async fn recv(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize, Infallible> {
        let n = buf.len().min(self.replies.len());
        for b in &mut buf[..n] {
            *b = self.replies.pop_front().unwrap();
        }
        if n < buf.len() {
            self.idle = self.idle.saturating_add(timeout);
        }
        Ok(n)
    }

    async fn delay(&mut self, duration: Duration) {
        self.idle = self.idle.saturating_add(duration);
    }
}
