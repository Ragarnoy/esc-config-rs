//! DShot beep command test.
//!
//! Tests DShot communication by sending beep commands at different speeds.
//! Cycles through DShot150, DShot300, DShot600, and DShot1200.
//!
//! # Usage
//! Flash this binary to the RP2350, connect GPIO0 to ESC signal wire,
//! power the ESC, and listen for beeps.

#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::peripherals::PIO0;
use embassy_rp::pio::{self, Pio};
use embassy_time::Timer;
use esc_dshot::{encode_command, Command, Speed};
use {defmt_rtt as _, panic_probe as _};

#[path = "../dshot_pio.rs"]
mod dshot_pio;

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => pio::InterruptHandler<PIO0>;
});

/// Send a DShot command per spec: 10x with telemetry, 10x without.
async fn send_command(dshot: &mut dshot_pio::DshotPio<'_, PIO0, 0>, cmd: Command) {
    let frame_tlm = encode_command(cmd, true);
    for _ in 0..10 {
        dshot.send_frame(frame_tlm).await;
        Timer::after_micros(1000).await;
    }

    let frame_no_tlm = encode_command(cmd, false);
    for _ in 0..10 {
        dshot.send_frame(frame_no_tlm).await;
        Timer::after_micros(1000).await;
    }
}

/// Send motor stop (arming/idle signal).
async fn send_idle(dshot: &mut dshot_pio::DshotPio<'_, PIO0, 0>, count: u32) {
    let frame = encode_command(Command::MotorStop, false);
    for _ in 0..count {
        dshot.send_frame(frame).await;
        Timer::after_micros(1000).await;
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("DShot Speed Test starting...");
    info!("Will test: DShot150 -> DShot300 -> DShot600 -> DShot1200");
    info!("Each speed: 3 beeps. Count beeps to see which speeds work.");

    let p = embassy_rp::init(Default::default());

    let Pio {
        mut common, sm0, ..
    } = Pio::new(p.PIO0, Irqs);

    info!("Connect GPIO0 to ESC signal wire");
    info!("Power on ESC and wait for init beeps to finish");
    info!("Starting in 3 seconds...");
    Timer::after_secs(3).await;

    // Test all speeds with distinct beep patterns:
    // DShot150:  1x Beep1 (lowest)
    // DShot300:  2x Beep2
    // DShot600:  3x Beep3
    // DShot1200: 4x Beep4 (highest)
    let speeds = [
        (Speed::DShot150, "DShot150", 1, Command::Beep1),
        (Speed::DShot300, "DShot300", 2, Command::Beep2),
        (Speed::DShot600, "DShot600", 3, Command::Beep3),
        (Speed::DShot1200, "DShot1200", 4, Command::Beep4),
    ];

    let pio_pin = common.make_pio_pin(p.PIN_0);

    // Start with DShot150 to test it without speed switching
    info!(">>> STARTING WITH DShot150 (no speed switch) <<<");
    let mut dshot = dshot_pio::DshotPio::new_with_pin(&mut common, sm0, pio_pin, Speed::DShot150);

    // Try DShot150 first, directly
    info!("=== Direct DShot150 test ===");
    info!("Arming at DShot150...");
    send_idle(&mut dshot, 2000).await;

    info!("Sending 3 beeps at DShot150...");
    for i in 1..=3u8 {
        info!("DShot150 direct - Beep {}/3", i);
        send_command(&mut dshot, Command::Beep1).await;
        send_idle(&mut dshot, 1000).await;
    }

    info!("DShot150 direct test complete.");
    info!(">>> 3 second pause before speed cycling <<<");
    Timer::after_secs(3).await;

    loop {
        for (speed, name, beep_count, beep_cmd) in speeds {
            info!("=== Testing {} ({} beeps) ===", name, beep_count);

            // Just change the clock divider, don't reload PIO program
            dshot.set_speed(speed);

            // Arm - longer to ensure ESC is ready
            info!("Arming...");
            send_idle(&mut dshot, 1500).await;

            // Send beeps with longer gaps
            for i in 1..=beep_count {
                info!("{} - Beep {}/{}", name, i, beep_count);
                send_command(&mut dshot, beep_cmd).await;
                // 1 second of idle between beeps
                send_idle(&mut dshot, 1000).await;
            }

            info!("{} done.", name);
            info!(">>> 3 second pause before next speed <<<");
            Timer::after_secs(3).await;
        }

        info!("=== Cycle complete! ===");
        info!("DShot150:  1 beep  (Beep1 - lowest)");
        info!("DShot300:  2 beeps (Beep2)");
        info!("DShot600:  3 beeps (Beep3)");
        info!("DShot1200: 4 beeps (Beep4 - highest)");
        info!(">>> 5 second pause, then restart <<<");
        Timer::after_secs(5).await;
    }
}
