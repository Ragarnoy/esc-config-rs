//! ESC Configurator Relay Firmware
//!
//! This firmware runs on an RP2350 and acts as a USB-to-ESC bridge for AM32/BLHeli
//! ESC configuration. It handles the timing-critical write sequence that requires
//! atomic execution without USB latency.
//!
//! # Pin assignments
//! - GPIO0: ESC signal wire (one-wire half-duplex UART at 19200 baud)
//! - USB: CDC serial to PC
//!
//! # AM32 Bootloader Protocol
//!
//! All commands use CRC-16/IBM (polynomial 0xA001, init 0x0000), sent LSB first.
//!
//! ## Read sequence (handled by passthrough):
//! 1. Set Address: `[FF 00 ADDR_H ADDR_L CRC_L CRC_H]` → ACK (0x30)
//! 2. Read: `[03 LEN CRC_L CRC_H]` → `[DATA...] [CRC_L CRC_H] [ACK]`
//!
//! ## Write sequence (handled atomically by firmware):
//! 1. Set Address: `[FF 00 ADDR_H ADDR_L CRC_L CRC_H]` → ACK (0x30)
//! 2. Set Buffer: `[FE 00 x256 LEN CRC_L CRC_H]` → **NO ACK**
//! 3. Payload: `[DATA...] [CRC_L CRC_H]` → ACK (0x30)
//! 4. Flash/Commit: `[01 01 CRC_L CRC_H]` → ACK (0x30)
//!
//! The Set Buffer command does NOT receive an ACK - this is critical!
//! The firmware detects write transactions and handles steps 1-3 atomically.
//!
//! # Usage
//! 1. Connect GPIO0 to ESC signal wire, share GND
//! 2. Power on the ESC
//! 3. Run CLI configurator

#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::peripherals::{PIO0, USB};
use embassy_rp::pio::{self, Pio};
use embassy_rp::usb::{self, Driver};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::Timer;
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use embassy_usb::{Builder, Config};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

mod onewire_uart;

/// CRC-16/IBM (polynomial 0xA001, init 0x0000)
fn crc16_ibm(buf: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &byte in buf {
        let mut xb = byte;
        for _ in 0..8 {
            if ((xb & 0x01) ^ (crc as u8 & 0x01)) != 0 {
                crc >>= 1;
                crc ^= 0xA001;
            } else {
                crc >>= 1;
            }
            xb >>= 1;
        }
    }
    crc
}

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => usb::InterruptHandler<USB>;
    PIO0_IRQ_0 => pio::InterruptHandler<PIO0>;
});

/// Max response size: 256 data + 2 CRC + 1 ACK = 259 bytes
const MAX_RESPONSE_SIZE: usize = 280;

/// Message with length and data buffer
#[derive(Clone)]
struct DataPacket {
    len: usize,
    data: [u8; MAX_RESPONSE_SIZE],
}

impl DataPacket {
    fn new(src: &[u8]) -> Self {
        let mut data = [0u8; MAX_RESPONSE_SIZE];
        let len = src.len().min(MAX_RESPONSE_SIZE);
        data[..len].copy_from_slice(&src[..len]);
        Self { len, data }
    }

    fn as_slice(&self) -> &[u8] {
        &self.data[..self.len]
    }
}

/// Channel for USB -> ESC data
static USB_TO_ESC: Channel<CriticalSectionRawMutex, DataPacket, 4> = Channel::new();

/// Channel for ESC -> USB data
static ESC_TO_USB: Channel<CriticalSectionRawMutex, DataPacket, 4> = Channel::new();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    // USB CDC Setup
    let driver = Driver::new(p.USB, Irqs);

    let mut config = Config::new(0x1209, 0x0001); // Use test VID/PID
    config.manufacturer = Some("ESC-Config");
    config.product = Some("ESC Relay");
    config.serial_number = Some("001");
    config.max_power = 100;
    config.max_packet_size_0 = 64;

    static CONFIG_DESC: StaticCell<[u8; 256]> = StaticCell::new();
    static BOS_DESC: StaticCell<[u8; 256]> = StaticCell::new();
    static MSOS_DESC: StaticCell<[u8; 256]> = StaticCell::new();
    static CONTROL_BUF: StaticCell<[u8; 64]> = StaticCell::new();

    let mut builder = Builder::new(
        driver,
        config,
        CONFIG_DESC.init([0; 256]),
        BOS_DESC.init([0; 256]),
        MSOS_DESC.init([0; 256]),
        CONTROL_BUF.init([0; 64]),
    );

    static CDC_STATE: StaticCell<State> = StaticCell::new();
    let cdc_state = CDC_STATE.init(State::new());
    let cdc_class = CdcAcmClass::new(&mut builder, cdc_state, 64);

    let usb = builder.build();
    spawner.spawn(usb_task(usb)).unwrap();

    // PIO one-wire UART for ESC communication
    let Pio {
        mut common,
        sm0,
        sm1,
        ..
    } = Pio::new(p.PIO0, Irqs);
    let pio_pin = common.make_pio_pin(p.PIN_0);
    let uart = onewire_uart::OneWireUart::new_with_pin(&mut common, sm0, sm1, pio_pin, 19200);

    // Start tasks
    let (cdc_tx, cdc_rx) = cdc_class.split();
    spawner.spawn(usb_rx_task(cdc_rx)).unwrap();
    spawner.spawn(usb_tx_task(cdc_tx)).unwrap();
    spawner.spawn(esc_task(uart)).unwrap();

    loop {
        Timer::after_secs(60).await;
    }
}

/// USB device task.
#[embassy_executor::task]
async fn usb_task(mut usb: embassy_usb::UsbDevice<'static, Driver<'static, USB>>) {
    usb.run().await;
}

/// Handle ESC communication via one-wire UART.
///
/// Aggregates USB packets and handles write transactions atomically.
#[embassy_executor::task]
async fn esc_task(mut uart: onewire_uart::OneWireUart<'static, PIO0, 0, 1>) {
    use embassy_time::{Duration, with_timeout};

    let mut rx_buf = [0u8; MAX_RESPONSE_SIZE];
    let mut tx_buf = [0u8; 300];
    let mut tx_len: usize;

    loop {
        let pkt = USB_TO_ESC.receive().await;
        tx_len = pkt.len;
        tx_buf[..tx_len].copy_from_slice(pkt.as_slice());

        // Aggregate multi-packet transfers (writes span multiple USB packets)
        let should_aggregate = tx_len == 64 || tx_buf[0] == 0xFF;
        if should_aggregate {
            loop {
                match with_timeout(Duration::from_millis(50), USB_TO_ESC.receive()).await {
                    Ok(pkt) => {
                        let new_len = tx_len + pkt.len;
                        if new_len <= tx_buf.len() {
                            tx_buf[tx_len..new_len].copy_from_slice(pkt.as_slice());
                            tx_len = new_len;
                        } else {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        }

        // Detect write transaction: set_addr (6) + write_hdr (3) + data
        let is_write_combo = tx_len >= 9
            && tx_buf[0] == 0xFF
            && tx_buf[1] == 0x00
            && tx_buf[6] == 0xFE;

        if is_write_combo {
            // Handle write atomically to avoid USB latency issues
            let mut resp_buf = [0u8; 8];
            let mut resp_len = 0usize;

            // Step 1: Set Address → ACK
            uart.write(&tx_buf[0..6]).await;
            let n = uart.read_exact_with_timeout(&mut rx_buf, 1, 50).await;
            if n < 1 || rx_buf[0] != 0x30 {
                warn!("Set addr failed");
                if n > 0 {
                    ESC_TO_USB.send(DataPacket::new(&rx_buf[..n])).await;
                }
                uart.switch_to_tx_idle();
                continue;
            }
            resp_buf[resp_len] = 0x30;
            resp_len += 1;

            // Step 2: Set Buffer [FE 00 00 30 CRC] → NO ACK
            let set_buffer_cmd: [u8; 4] = [0xFE, 0x00, 0x00, 0x30];
            let set_buffer_crc = crc16_ibm(&set_buffer_cmd);
            let set_buffer_pkt: [u8; 6] = [
                0xFE, 0x00, 0x00, 0x30,
                set_buffer_crc as u8,
                (set_buffer_crc >> 8) as u8,
            ];
            Timer::after_millis(1).await;
            uart.write(&set_buffer_pkt).await;

            // Step 3: Payload (48 data + CRC) → ACK
            let data_start = 9usize;
            let data_available = tx_len.saturating_sub(data_start);
            if data_available < 48 {
                warn!("Incomplete write data");
                ESC_TO_USB.send(DataPacket::new(&resp_buf[..resp_len])).await;
                uart.switch_to_tx_idle();
                continue;
            }

            let mut payload = [0u8; 50];
            payload[0..48].copy_from_slice(&tx_buf[data_start..data_start+48]);
            let data_crc = crc16_ibm(&payload[0..48]);
            payload[48] = data_crc as u8;
            payload[49] = (data_crc >> 8) as u8;

            Timer::after_millis(1).await;
            uart.write(&payload).await;

            let n2 = uart.read_exact_with_timeout(&mut rx_buf, 1, 100).await;
            if n2 > 0 {
                resp_buf[resp_len] = rx_buf[0];
                resp_len += 1;
            }

            ESC_TO_USB.send(DataPacket::new(&resp_buf[..resp_len])).await;
            uart.switch_to_tx_idle();
            continue;
        }

        // Normal passthrough path
        uart.write(&tx_buf[..tx_len]).await;
        let n = uart.read_with_timeout(&mut rx_buf, 200).await;
        if n > 0 {
            ESC_TO_USB.send(DataPacket::new(&rx_buf[..n])).await;
        }
        uart.switch_to_tx_idle();
    }
}

/// Receive data from USB and send to ESC channel.
#[embassy_executor::task]
async fn usb_rx_task(
    mut cdc_rx: embassy_usb::class::cdc_acm::Receiver<'static, Driver<'static, USB>>,
) {
    let mut buf = [0u8; 64];
    loop {
        cdc_rx.wait_connection().await;
        loop {
            match cdc_rx.read_packet(&mut buf).await {
                Ok(n) if n > 0 => {
                    USB_TO_ESC.send(DataPacket::new(&buf[..n])).await;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    }
}

/// Send data from ESC channel to USB.
#[embassy_executor::task]
async fn usb_tx_task(
    mut cdc_tx: embassy_usb::class::cdc_acm::Sender<'static, Driver<'static, USB>>,
) {
    loop {
        cdc_tx.wait_connection().await;
        'connected: loop {
            let pkt = ESC_TO_USB.receive().await;
            for chunk in pkt.as_slice().chunks(64) {
                if cdc_tx.write_packet(chunk).await.is_err() {
                    break 'connected;
                }
            }
        }
    }
}
