//! ESC configurator relay firmware (RP2350).
//!
//! Runs the AM32 bootloader session (`esc-session`) on the ESC's signal wire and
//! serves whole operations (connect, read/write settings, flash chunks, run) to the
//! PC as postcard-rpc endpoints (`esc-config-icd`) over USB CDC-ACM, COBS-framed.
//! Nothing protocol-level crosses USB, so USB latency cannot split a bootloader
//! sequence.
//!
//! # Pins
//! - GPIO15: ESC signal wire (PIO one-wire UART, 19200 8N1, idles high)
//! - USB: CDC-ACM, VID:PID 1209:0001 (pid.codes test)
//!
//! # Usage
//! Power the relay first, then the ESC: a signal line driven high at ESC power-on
//! keeps the AM32 bootloader active. Then run `esc-configurator`.

#![no_std]
#![no_main]

mod app;
mod frame_buf;
mod wire;

use defmt::info;
use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::peripherals::{PIO0, USB};
use embassy_rp::pio::{self, Pio};
use embassy_rp::usb::{self, Driver};
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use embassy_usb::{Builder, Config, UsbDevice};
use esc_onewire_rp::OneWireUart;
use postcard_rpc::server::{Dispatch, Server};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

use crate::app::{Context, RelayApp};
use crate::wire::{CdcRx, CdcTx, FRAME_SIZE, NoSpawn, PACKET_SIZE};

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => usb::InterruptHandler<USB>;
    PIO0_IRQ_0 => pio::InterruptHandler<PIO0>;
});

#[embassy_executor::main(
    executor = "embassy_rp::executor::Executor",
    entry = "cortex_m_rt::entry"
)]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    // ESC signal line first, so it is driven high as early as possible.
    let Pio {
        mut common,
        sm0,
        sm1,
        ..
    } = Pio::new(p.PIO0, Irqs);
    let uart = OneWireUart::new(&mut common, sm0, sm1, p.PIN_15);

    // USB CDC-ACM.
    let driver = Driver::new(p.USB, Irqs);
    let mut config = Config::new(0x1209, 0x0001);
    config.manufacturer = Some("esc-config-rs");
    config.product = Some("ESC Relay (ICD)");
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
    let class = CdcAcmClass::new(
        &mut builder,
        CDC_STATE.init(State::new()),
        PACKET_SIZE as u16,
    );
    let usb = builder.build();
    spawner.spawn(usb_task(usb).unwrap());

    let (cdc_tx, cdc_rx) = class.split();
    static RX_BUF: StaticCell<[u8; FRAME_SIZE]> = StaticCell::new();
    let rx_buf: &'static mut [u8] = RX_BUF.init([0; FRAME_SIZE]);
    let dispatcher = RelayApp::new(
        Context {
            uart,
            connected: None,
        },
        NoSpawn,
    );
    let vkk = dispatcher.min_key_len();
    let mut server = Server::new(
        CdcTx::new(cdc_tx),
        CdcRx::new(cdc_rx),
        rx_buf,
        dispatcher,
        vkk,
    );

    info!("ESC relay ready");
    loop {
        // Returns when the host disconnects; wait for the next connection.
        let _ = server.run().await;
    }
}

/// USB device task.
#[embassy_executor::task]
async fn usb_task(mut usb: UsbDevice<'static, Driver<'static, USB>>) {
    usb.run().await;
}
