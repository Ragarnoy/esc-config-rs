//! Session behaviour against a simulated AM32 bootloader.
//!
//! `Fnn`/`Pn` refer to the Phase 1 findings in `docs/REVAMP.md`.

mod fake_esc;

use embassy_futures::block_on;
use esc_protocol::device::BootloaderType;
use esc_session::{Duration, EEPROM_SIZE, Error, Esc, Timing};
use fake_esc::{BOOTLOADER_VERSION, FakeEsc, Mcu};

fn connect(fake: FakeEsc) -> Esc<FakeEsc> {
    block_on(Esc::connect(fake)).expect("connect")
}

fn image(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 7 + 3) as u8).collect()
}

// ----------------------------------------------------------------------------
// Connect and layout
// ----------------------------------------------------------------------------

#[test]
fn connect_reads_layout_from_devinfo() {
    let esc = connect(FakeEsc::new(Mcu::F051));
    let l = esc.layout();
    assert_eq!(
        (
            l.address_shift,
            l.firmware_start,
            l.filename_start,
            l.eeprom_start
        ),
        (0, 0x1000, 0x7BE0, 0x7C00)
    );
    assert_eq!(esc.device().pin_code(), 0x06);
}

#[test]
fn connect_falls_back_to_flash_code_on_old_bootloaders() {
    let mut fake = FakeEsc::new(Mcu::G071);
    fake.protocol_version = 1;
    let esc = connect(fake);
    assert_eq!(esc.layout().address_shift, 2);
    assert_eq!(esc.layout().eeprom_start, 0x7E00);
    // No devinfo read was attempted.
    let fake = esc.into_link();
    assert!(
        !fake
            .frames
            .iter()
            .any(|f| f[..4] == [0xFF, 0x00, 0x00, 0x23])
    );
}

#[test]
fn connect_finds_dronecan_app_start() {
    // F5: a DroneCAN bootloader's app starts at 0x4000; only devinfo says so.
    let esc = connect(FakeEsc::dronecan(Mcu::G071));
    assert_eq!(esc.layout().firmware_byte_offset(), 0x4000);
}

#[test]
fn connect_accepts_any_signal_pin() {
    // F7: PB4 is 0x14, previously rejected as "unknown".
    let mut fake = FakeEsc::new(Mcu::F421);
    fake.pin_code = 0x14;
    let esc = connect(fake);
    assert_eq!(esc.device().device_id, 0x3514);
}

#[test]
fn connect_refuses_unknown_flash_code() {
    let mut fake = FakeEsc::new(Mcu::F051);
    fake.mcu.flash_code = 0x33;
    match block_on(Esc::connect(fake)) {
        Err(Error::Unsupported(info)) => assert_eq!(info.bootloader, BootloaderType::Unknown),
        other => panic!("expected Unsupported, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn connect_retries_then_gives_up() {
    let mut fake = FakeEsc::new(Mcu::F051);
    fake.drop_replies = 2;
    connect(fake); // third attempt answers

    let mut fake = FakeEsc::new(Mcu::F051);
    fake.drop_replies = 3;
    assert!(matches!(block_on(Esc::connect(fake)), Err(Error::NoReply)));
}

// ----------------------------------------------------------------------------
// Reads
// ----------------------------------------------------------------------------

#[test]
fn reads_are_crc_checked_and_retried() {
    // F3: a corrupted read used to be accepted.
    let mut esc = connect(FakeEsc::new(Mcu::F051));
    let expected = esc_eeprom(&mut esc);

    let mut fake = esc.into_link();
    fake.corrupt_reads = 2;
    let mut esc = connect(fake);
    assert_eq!(block_on(esc.read_eeprom()).unwrap(), expected);

    let mut fake = esc.into_link();
    fake.corrupt_reads = 10;
    let mut esc = connect(fake);
    assert_eq!(block_on(esc.read_eeprom()), Err(Error::ReadCrc));
}

fn esc_eeprom(esc: &mut Esc<FakeEsc>) -> [u8; EEPROM_SIZE] {
    block_on(esc.read_eeprom()).unwrap()
}

#[test]
fn file_name_on_g071_is_read_at_the_shifted_address() {
    // F6: `eeprom - 32` in protocol units is 128 bytes off on G071.
    let mut esc = connect(FakeEsc::new(Mcu::G071));
    let name = block_on(esc.read_file_name()).unwrap();
    assert!(name.starts_with(b"AM32_TEST_ESC\0"));
}

// ----------------------------------------------------------------------------
// Settings writes
// ----------------------------------------------------------------------------

#[test]
fn eeprom_write_keeps_melody_and_can() {
    // F2: the 176-byte write erased the page and lost CAN bytes 176..184.
    for mcu in [Mcu::F051, Mcu::F421, Mcu::G071] {
        let mut esc = connect(FakeEsc::new(mcu));
        let mut block = esc_eeprom(&mut esc);
        block[0x17] = 0x42; // timing advance
        block_on(esc.write_eeprom(&block)).unwrap();

        let fake = esc.into_link();
        let stored = fake.eeprom();
        assert_eq!(stored[0x17], 0x42);
        assert_eq!(&stored[176..184], &[7, 1, 1, 50, 0, 0, 0, 1]);
        assert!(stored[48..176].iter().all(|&b| b & 0x80 != 0));
        assert_eq!(stored[2], BOOTLOADER_VERSION);
    }
}

#[test]
fn bad_payload_is_resent() {
    // F8: after 0xC2 the bootloader is still waiting for the payload.
    let mut fake = FakeEsc::new(Mcu::F051);
    let mut block = [0u8; EEPROM_SIZE];
    block[..EEPROM_SIZE].copy_from_slice(&fake.flash[0x7C00..0x7C00 + EEPROM_SIZE]);
    block[5] = 99;
    fake.corrupt_payloads = 2;
    let mut esc = connect(fake);
    block_on(esc.write_eeprom(&block)).unwrap();
    assert_eq!(esc.into_link().eeprom()[5], 99);
}

#[test]
fn lost_payload_ack_is_not_resent_as_a_command() {
    // The payload was taken but its ACK lost: the bootloader is out of payload
    // mode, so resending the payload would run its bytes as a command (here 0x01,
    // a Prog Flash with a bad CRC, answered 0xC2 on every retry).
    let mut fake = FakeEsc::new(Mcu::F051);
    let mut block = [0u8; EEPROM_SIZE];
    block.copy_from_slice(&fake.flash[0x7C00..0x7C00 + EEPROM_SIZE]);
    block[7] = 55;
    // Replies: connect (1), devinfo (2, 3), then Set Address (4), payload (5).
    fake.drop_reply_number = Some(5);
    let mut esc = connect(fake);
    block_on(esc.write_eeprom(&block)).unwrap();
    let fake = esc.into_link();
    assert_eq!(fake.eeprom()[7], 55);
    assert_eq!(fake.invalid_commands, 0, "no payload byte ran as a command");
}

#[test]
fn lost_payload_is_resent() {
    // The payload runs into the Set Buffer frame (gap too short) and is lost. The
    // Set Address probe is taken as a payload and ends payload mode; Set Buffer
    // and the payload then go again.
    let mut esc = connect(FakeEsc::new(Mcu::F051));
    let mut block = esc_eeprom(&mut esc);
    block[8] = 66;
    let mut fake = esc.into_link();
    fake.merge_next_payload = true;
    let mut esc = connect(fake);
    block_on(esc.write_eeprom(&block)).unwrap();
    assert_eq!(esc.into_link().eeprom()[8], 66);
}

#[test]
fn failed_write_leaves_payload_mode() {
    // Every payload fails its CRC: the write gives up. Left in payload mode, the
    // bootloader would ACK the next Set Address as a payload and the next read
    // would come from the failed write's address instead of the EEPROM.
    let mut fake = FakeEsc::new(Mcu::F051);
    fake.corrupt_payloads = 4;
    let mut esc = connect(fake);
    assert_eq!(
        block_on(esc.write_at(0x2000, &[0x11; 64])),
        Err(Error::Nack(0xC2))
    );
    let eeprom = esc_eeprom(&mut esc);
    assert_eq!(&eeprom[..], esc.into_link().eeprom());
}

#[test]
fn set_buffer_needs_an_idle_gap() {
    // Without the gap the payload runs into the Set Buffer frame and is lost.
    let timing = Timing {
        set_buffer_gap: Duration::ZERO,
        retries: 0,
        ..Timing::default()
    };
    let mut esc = block_on(Esc::connect_with(FakeEsc::new(Mcu::F051), timing)).unwrap();
    let block = esc_eeprom(&mut esc);
    assert_eq!(block_on(esc.write_eeprom(&block)), Err(Error::NoReply));
}

#[test]
fn full_256_byte_writes_work() {
    // P6: FE 00 01 00 announces 256 bytes; FE 00 00 00 announced zero.
    let mut esc = connect(FakeEsc::new(Mcu::F051));
    let data = image(256);
    block_on(esc.write_at(0x2000, &data)).unwrap();
    let mut back = [0u8; 256];
    block_on(esc.read_at(0x2000, &mut back)).unwrap();
    assert_eq!(back.as_slice(), data.as_slice());
}

#[test]
fn writes_must_be_a_multiple_of_8() {
    let mut esc = connect(FakeEsc::new(Mcu::G071));
    assert_eq!(
        block_on(esc.write_at(0x1000, &[0; 12])),
        Err(Error::InvalidLength(12))
    );
    assert_eq!(
        block_on(esc.write_at(0x1000, &[0; 264])),
        Err(Error::InvalidLength(264))
    );
}

#[test]
fn lost_prog_flash_ack_is_retried() {
    let mut fake = FakeEsc::new(Mcu::F051);
    let mut block = [0u8; EEPROM_SIZE];
    block.copy_from_slice(&fake.flash[0x7C00..0x7C00 + EEPROM_SIZE]);
    block[6] = 77;
    // Replies: connect (1), devinfo Set Address (2) and read (3), then the write's
    // Set Address (4), payload (5) and Prog Flash (6).
    fake.drop_reply_number = Some(6);
    let mut esc = connect(fake);
    block_on(esc.write_eeprom(&block)).unwrap();
    assert_eq!(esc.into_link().eeprom()[6], 77);
}

// ----------------------------------------------------------------------------
// Firmware
// ----------------------------------------------------------------------------

fn flash_and_check(mcu: Mcu, len: usize) {
    let mut esc = connect(FakeEsc::new(mcu));
    let img = image(len);
    let mut calls = 0;
    block_on(esc.flash_firmware(&img, |done, total| {
        calls += 1;
        assert!(done <= total);
    }))
    .unwrap();
    assert_eq!(calls, len.div_ceil(256));
    block_on(esc.verify_firmware(&img, |_, _| {})).unwrap();

    let fake = esc.into_link();
    let start = fake.mcu.app_start;
    assert_eq!(&fake.flash[start..start + len], img.as_slice());
    // F9: the tail is padded with 0xFF to a multiple of 8.
    let padded = len.next_multiple_of(8);
    assert!(
        fake.flash[start + len..start + padded]
            .iter()
            .all(|&b| b == 0xFF)
    );
    assert_eq!(fake.eeprom()[0], 0x01, "bootable after flashing");
}

#[test]
fn flash_f051() {
    flash_and_check(Mcu::F051, 20_003);
}

#[test]
fn flash_g071_uses_shifted_addresses() {
    // F5: G071 addresses are divided by 4; the app must land at 0x1000.
    flash_and_check(Mcu::G071, 70_001);
}

#[test]
fn flash_dronecan_starts_at_0x4000() {
    let mut esc = connect(FakeEsc::dronecan(Mcu::F421));
    let img = image(4096);
    block_on(esc.flash_firmware(&img, |_, _| {})).unwrap();
    let fake = esc.into_link();
    assert_eq!(&fake.flash[0x4000..0x4000 + 4096], img.as_slice());
}

#[test]
fn flash_over_existing_app_erases_pages() {
    let mut esc = connect(FakeEsc::new(Mcu::F051));
    block_on(esc.flash_firmware(&image(3000), |_, _| {})).unwrap();
    let img: Vec<u8> = image(3000).iter().map(|b| !b).collect();
    block_on(esc.flash_firmware(&img, |_, _| {})).unwrap();
    block_on(esc.verify_firmware(&img, |_, _| {})).unwrap();
}

#[test]
fn failed_flash_leaves_the_esc_in_the_bootloader() {
    let mut fake = FakeEsc::new(Mcu::F051);
    fake.fail_prog_in = Some(0x1800..0x7C00);
    let mut esc = connect(fake);
    let err = block_on(esc.flash_firmware(&image(8192), |_, _| {})).unwrap_err();
    assert_eq!(err, Error::Nack(0xC1));
    let fake = esc.into_link();
    assert_eq!(fake.eeprom()[0], 0x00, "boot byte must stay cleared");
}

#[test]
fn image_must_fit_below_the_eeprom() {
    let mut esc = connect(FakeEsc::new(Mcu::F051));
    let max = esc.max_firmware_size();
    assert_eq!(max, 0x7C00 - 0x1000);
    assert_eq!(
        block_on(esc.flash_firmware(&image(max + 1), |_, _| {})),
        Err(Error::ImageTooLarge { size: max + 1, max })
    );
    assert_eq!(
        block_on(esc.flash_firmware(&[], |_, _| {})),
        Err(Error::InvalidLength(0))
    );
}

#[test]
fn verify_reports_the_first_bad_chunk() {
    let mut esc = connect(FakeEsc::new(Mcu::F051));
    let img = image(1024);
    block_on(esc.flash_firmware(&img, |_, _| {})).unwrap();
    let mut other = img.clone();
    other[600] ^= 1;
    assert_eq!(
        block_on(esc.verify_firmware(&other, |_, _| {})),
        Err(Error::Verify { offset: 512 })
    );
}

// ----------------------------------------------------------------------------
// Leaving the bootloader
// ----------------------------------------------------------------------------

#[test]
fn run_starts_the_app() {
    // F13: `00 01` was answered 0xC1 and ignored; Run is `00 00 00 00`.
    let esc = connect(FakeEsc::new(Mcu::F051));
    let fake = block_on(esc.run()).unwrap();
    assert!(fake.started_app);
    assert_eq!(fake.frames.last().unwrap().as_slice(), &[0, 0, 0, 0]);
}

#[test]
fn run_does_not_start_an_app_with_boot_byte_cleared() {
    let mut fake = FakeEsc::new(Mcu::F051);
    fake.flash[0x7C00] = 0x00;
    let fake = block_on(connect(fake).run()).unwrap();
    assert!(!fake.started_app);
}

// ----------------------------------------------------------------------------
// Stepwise flashing (what a server drives one request at a time)
// ----------------------------------------------------------------------------

#[test]
fn stepwise_flash_over_a_borrowed_link() {
    let mut fake = FakeEsc::new(Mcu::G071);
    let img = image(1000);
    {
        let mut esc = block_on(Esc::connect(&mut fake)).unwrap();
        block_on(esc.begin_flash()).unwrap();
        block_on(esc.begin_flash()).unwrap(); // idempotent
        for (i, chunk) in img.chunks(256).enumerate() {
            block_on(esc.write_firmware_chunk(i * 256, chunk)).unwrap();
        }
        assert_eq!(esc_eeprom_byte0(&mut esc), 0x00);
        block_on(esc.finish_flash()).unwrap();
    }
    // The caller still owns the link.
    assert_eq!(&fake.flash[0x1000..0x1000 + 1000], img.as_slice());
    assert_eq!(fake.eeprom()[0], 0x01);
}

fn esc_eeprom_byte0<L: esc_session::Link>(esc: &mut Esc<L>) -> u8
where
    L::Error: core::fmt::Debug,
{
    block_on(esc.read_eeprom()).unwrap()[0]
}

#[test]
fn chunk_bounds_and_alignment_are_checked() {
    let mut esc = connect(FakeEsc::new(Mcu::F051));
    let max = esc.max_firmware_size();
    assert_eq!(
        block_on(esc.write_firmware_chunk(4, &[0; 8])),
        Err(Error::BadAddress)
    );
    assert_eq!(
        block_on(esc.write_firmware_chunk(max - 8, &[0; 16])),
        Err(Error::ImageTooLarge { size: max + 8, max })
    );
    assert_eq!(
        block_on(esc.write_firmware_chunk(0, &[])),
        Err(Error::InvalidLength(0))
    );
    // An offset that would wrap is refused before anything is sent.
    let sent = esc.into_link().frames.len();
    let mut esc = connect(FakeEsc::new(Mcu::F051));
    assert_eq!(
        block_on(esc.write_firmware_chunk(usize::MAX - 7, &[0; 16])),
        Err(Error::BadAddress)
    );
    assert!(esc.into_link().frames.len() <= sent, "nothing written");
}

#[test]
fn finish_without_begin_is_harmless() {
    let mut esc = connect(FakeEsc::new(Mcu::F051));
    block_on(esc.finish_flash()).unwrap();
    assert_eq!(esc.into_link().eeprom()[0], 0x01);
}

#[test]
fn failed_connect_leaves_the_link_with_the_caller() {
    let mut fake = FakeEsc::new(Mcu::F051);
    fake.drop_replies = 3;
    assert!(block_on(Esc::connect(&mut fake)).is_err());
    // Same link, second try.
    block_on(Esc::connect(&mut fake)).unwrap();
}
