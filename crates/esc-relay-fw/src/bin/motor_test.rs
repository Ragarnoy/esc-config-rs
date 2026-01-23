//! Motor spin test binary.
//!
//! This standalone binary tests motor control via DShot.
//! Use this for validating that DShot communication works before
//! trying bootloader communication.
//!
//! # Usage
//! Flash this binary to the RP2350, connect GPIO0 to ESC signal wire,
//! power the ESC, and observe motor behavior.

#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::peripherals::PIO0;
use embassy_rp::pio::{self, Pio};
use embassy_time::Timer;
use esc_dshot::{frame, Speed};
use {defmt_rtt as _, panic_probe as _};

#[path = "../dshot_pio.rs"]
mod dshot_pio;

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => pio::InterruptHandler<PIO0>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("Motor Test starting...");

    let p = embassy_rp::init(Default::default());

    // -------------------------------------------------------------------------
    // PIO Setup
    // -------------------------------------------------------------------------
    let Pio {
        mut common, sm0, ..
    } = Pio::new(p.PIO0, Irqs);

    info!("Connect GPIO0 to ESC signal wire");

    let pio_pin = common.make_pio_pin(p.PIN_0);
    let mut dshot = dshot_pio::DshotPio::new_with_pin(&mut common, sm0, pio_pin, Speed::DShot600);

    // -------------------------------------------------------------------------
    // Motor spin test
    // -------------------------------------------------------------------------
    info!("Sending DShot - motor should spin!");
    info!("First: arming sequence (throttle=0 for 1 second)");

    // Arm: send throttle=0 for a bit (command 0 = motor stop)
    let arm_frame = frame::encode(0, false);
    for _ in 0..2000 {
        dshot.send_frame(arm_frame).await;
        Timer::after_micros(1000).await;
    }

    info!("Armed. Now ramping up throttle...");

    const MAX_THROTTLE: u16 = 850;

    // Slowly ramp up throttle (48 is minimum throttle, below that are commands)
    for throttle in (48..=MAX_THROTTLE).step_by(1) {
        let motor_frame = frame::encode(throttle, false);
        if throttle % 50 == 0 {
            info!("Throttle: {}", throttle);
        }

        for _ in 0..50 {
            dshot.send_frame(motor_frame).await;
            Timer::after_micros(500).await;
        }
    }

    info!("Holding throttle at {}...", MAX_THROTTLE);

    // Hold at max throttle
    let hold_frame = frame::encode(MAX_THROTTLE, false);
    for _ in 0..9000 {
        dshot.send_frame(hold_frame).await;
        Timer::after_micros(500).await;
    }

    info!("Ramping down throttle...");

    // Ramp down throttle
    for throttle in (48..=MAX_THROTTLE).rev().step_by(2) {
        let motor_frame = frame::encode(throttle, false);
        if throttle % 50 == 0 {
            info!("Throttle: {}", throttle);
        }

        for _ in 0..50 {
            dshot.send_frame(motor_frame).await;
            Timer::after_micros(500).await;
        }
    }

    info!("Stopping motor...");

    // Stop
    let stop_frame = frame::encode(0, false);
    for _ in 0..2000 {
        dshot.send_frame(stop_frame).await;
        Timer::after_micros(500).await;
    }

    info!("Done! Motor test complete.");

    loop {
        Timer::after_secs(60).await;
    }
}
