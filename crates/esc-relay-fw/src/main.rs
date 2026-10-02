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

#[embassy_executor::main(
    executor = "embassy_rp::executor::Executor",
    entry = "cortex_m_rt::entry"
)]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    // USB CDC Setup
    let driver = Driver::new(p.USB, Irqs);

    // VID 0x0403 (FTDI) so am32.ca detects this as a direct-connect device
    // and speaks AM32 bootloader protocol directly instead of MSP/4way-if.
    // Using a PID (0x0001) not in the ftdi_sio driver's ID table so Linux
    // uses the generic cdc_acm driver instead of fighting with ftdi_sio.
    // (Previously used CH340 VID 0x1A86 but Linux's ch341 driver claimed it.)
    let mut config = Config::new(0x0403, 0x0001);
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
    spawner.spawn(usb_task(usb).unwrap());

    // PIO one-wire UART for ESC communication
    let Pio {
        mut common,
        sm0,
        sm1,
        ..
    } = Pio::new(p.PIO0, Irqs);
    let pio_pin = common.make_pio_pin(p.PIN_15);
    let uart = onewire_uart::OneWireUart::new_with_pin(&mut common, sm0, sm1, pio_pin, 19200);

    // Start tasks
    let (cdc_tx, cdc_rx) = cdc_class.split();
    spawner.spawn(usb_rx_task(cdc_rx).unwrap());
    spawner.spawn(usb_tx_task(cdc_tx).unwrap());
    spawner.spawn(esc_task(uart).unwrap());

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

        // Aggregate USB packets: Web Serial API / USB CDC can fragment writes
        // at arbitrary boundaries. Wait briefly for more data to reassemble
        // complete protocol commands before forwarding to ESC.
        // 2ms is enough to catch fragments from a single write (~1ms USB latency)
        // but short enough to not merge separate sequential commands (~20ms+ apart).
        while let Ok(pkt) = with_timeout(Duration::from_millis(2), USB_TO_ESC.receive()).await {
            let new_len = tx_len + pkt.len;
            if new_len > tx_buf.len() {
                break;
            }
            tx_buf[tx_len..new_len].copy_from_slice(pkt.as_slice());
            tx_len = new_len;
        }

        // Detect write transaction: set_addr (6) + set_buffer (6) + data + crc (2+)
        // CLI batches these together. Strict check to avoid false matches on raw data.
        let is_write_combo = tx_len >= 14
            && tx_buf[0] == 0xFF
            && tx_buf[1] == 0x00
            && tx_buf[6] == 0xFE
            && tx_buf[7] == 0x00
            && tx_buf[8] == 0x00;

        if is_write_combo {
            // Handle write atomically to avoid USB latency issues
            //
            // CLI sends: [set_addr:6] [set_buffer:6] [data:N] [data_crc:2]
            // - set_addr:   [FF 00 ADDR_H ADDR_L CRC_L CRC_H]
            // - set_buffer: [FE 00 00 SIZE CRC_L CRC_H]
            // - data + crc: [DATA...] [CRC_L CRC_H]
            //
            // We need to:
            // 1. Send set_addr to ESC → expect ACK
            // 2. Send set_buffer to ESC → NO ACK (AM32 quirk)
            // 3. Send data + our CRC to ESC → expect ACK

            // Echo transmitted bytes back to host first
            ESC_TO_USB.send(DataPacket::new(&tx_buf[..tx_len])).await;

            let mut resp_buf = [0u8; 8];
            let mut resp_len = 0usize;

            // Parse size from set_buffer command (tx_buf[9])
            // SIZE=0 means 256 bytes in AM32 protocol
            let size = if tx_buf[9] == 0 {
                256usize
            } else {
                tx_buf[9] as usize
            };
            let data_start = 12usize; // After set_addr (6) + set_buffer (6)
            let data_available = tx_len.saturating_sub(data_start + 2); // -2 for CLI's CRC

            if data_available < size {
                warn!(
                    "Incomplete write data: have {}, need {}",
                    data_available, size
                );
                ESC_TO_USB
                    .send(DataPacket::new(&resp_buf[..resp_len]))
                    .await;
                uart.switch_to_tx_idle();
                continue;
            }

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

            // Step 2: Set Buffer → NO ACK
            // Use the CLI's set_buffer command directly (already has correct size + CRC)
            Timer::after_millis(1).await;
            uart.write(&tx_buf[6..12]).await;

            // Step 3: Payload (data + our CRC) → ACK
            // Compute our own CRC for the data
            let data_crc = crc16_ibm(&tx_buf[data_start..data_start + size]);

            Timer::after_millis(1).await;
            uart.write(&tx_buf[data_start..data_start + size]).await;
            let crc_bytes = [data_crc as u8, (data_crc >> 8) as u8];
            uart.write(&crc_bytes).await;

            // Longer timeout for larger writes (flash operations take time)
            let timeout_ms = if size > 64 { 200 } else { 100 };
            let n2 = uart
                .read_exact_with_timeout(&mut rx_buf, 1, timeout_ms)
                .await;
            if n2 > 0 {
                resp_buf[resp_len] = rx_buf[0];
                resp_len += 1;
            }

            ESC_TO_USB
                .send(DataPacket::new(&resp_buf[..resp_len]))
                .await;
            uart.switch_to_tx_idle();
            continue;
        }

        // Check if this is a set_buffer command (0xFE) - NO ACK expected
        // Strict check: exactly 6 bytes [0xFE, 0x00, 0x00, SIZE, CRC_L, CRC_H]
        // to avoid false matches on raw firmware data starting with 0xFE.
        //
        // After SetBuffer, the ESC expects the data payload immediately.
        // We must handle SetBuffer → Data atomically to avoid USB latency
        // causing the ESC to timeout between the two commands.
        let is_set_buffer =
            tx_len == 6 && tx_buf[0] == 0xFE && tx_buf[1] == 0x00 && tx_buf[2] == 0x00;
        if is_set_buffer {
            // Parse expected data size from SetBuffer command
            let data_size = if tx_buf[3] == 0 {
                256usize
            } else {
                tx_buf[3] as usize
            };

            // Echo the SetBuffer command
            ESC_TO_USB.send(DataPacket::new(&tx_buf[..tx_len])).await;

            // Wait for the data payload from USB (data + CRC = data_size + 2 bytes)
            let expected = data_size + 2;
            let mut data_buf = [0u8; 300];
            let mut data_len = 0usize;
            let data_deadline = Duration::from_millis(500);
            let deadline_start = embassy_time::Instant::now();

            while data_len < expected {
                let remaining = data_deadline
                    .checked_sub(deadline_start.elapsed())
                    .unwrap_or(Duration::from_ticks(0));
                if remaining.as_ticks() == 0 {
                    break;
                }
                match with_timeout(remaining, USB_TO_ESC.receive()).await {
                    Ok(pkt) => {
                        let new_len = (data_len + pkt.len).min(data_buf.len());
                        data_buf[data_len..new_len]
                            .copy_from_slice(&pkt.as_slice()[..new_len - data_len]);
                        data_len = new_len;
                    }
                    Err(_) => break,
                }
            }

            if data_len < expected {
                warn!(
                    "SetBuffer: incomplete data, got {} expected {}",
                    data_len, expected
                );
                uart.switch_to_tx_idle();
                continue;
            }

            // Send SetBuffer to ESC (no ACK expected)
            uart.write(&tx_buf[..6]).await;

            // Send data + CRC to ESC atomically after SetBuffer
            // Recompute CRC from the actual data (ignore client CRC)
            let data_crc = crc16_ibm(&data_buf[..data_size]);
            Timer::after_millis(1).await;
            uart.write(&data_buf[..data_size]).await;
            let crc_bytes = [data_crc as u8, (data_crc >> 8) as u8];
            uart.write(&crc_bytes).await;

            // Wait for ESC ACK
            let timeout_ms = if data_size > 64 { 200 } else { 100 };
            let n = uart
                .read_exact_with_timeout(&mut rx_buf, 1, timeout_ms)
                .await;

            // Echo the data payload + ESC response
            let mut resp = [0u8; MAX_RESPONSE_SIZE];
            let echo_len = data_len.min(MAX_RESPONSE_SIZE);
            resp[..echo_len].copy_from_slice(&data_buf[..echo_len]);
            let resp_n = n.min(MAX_RESPONSE_SIZE - echo_len);
            if resp_n > 0 {
                resp[echo_len..echo_len + resp_n].copy_from_slice(&rx_buf[..resp_n]);
            }
            ESC_TO_USB
                .send(DataPacket::new(&resp[..echo_len + resp_n]))
                .await;
            uart.switch_to_tx_idle();
            continue;
        }

        // Normal passthrough path
        // Echo transmitted bytes back to host (one-wire: TX appears on RX)
        // then append the ESC's response.
        // Use 500ms initial timeout for slow operations (page erase, flash commit).
        info!("PASS: {} bytes, first=0x{:02X}", tx_len, tx_buf[0]);
        uart.write(&tx_buf[..tx_len]).await;
        let n = uart.read_with_timeout(&mut rx_buf, 500).await;
        info!("PASS: got {} bytes back", n);

        // Build combined response: echo + ESC response
        let mut combined = [0u8; MAX_RESPONSE_SIZE];
        let echo_len = tx_len.min(MAX_RESPONSE_SIZE);
        combined[..echo_len].copy_from_slice(&tx_buf[..echo_len]);
        let resp_len = n.min(MAX_RESPONSE_SIZE - echo_len);
        combined[echo_len..echo_len + resp_len].copy_from_slice(&rx_buf[..resp_len]);
        ESC_TO_USB
            .send(DataPacket::new(&combined[..echo_len + resp_len]))
            .await;
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
