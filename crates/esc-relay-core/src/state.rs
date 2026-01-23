//! Relay state machine.
//!
//! The relay operates in three states:
//! 1. **Idle**: Waiting for commands from CLI
//! 2. **DShot**: Sending DShot frames to ESC (bootloader entry)
//! 3. **Passthrough**: Transparent bridge between USB and UART

use crate::protocol::{Command, CommandOp, DshotSpeed, Response, Status};
use crate::traits::{DelayNs, DshotOutput, Read, Write};
use esc_dshot::{encode, Command as DshotCommand};

/// Relay operating state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum RelayState {
    /// Idle, waiting for commands.
    Idle,

    /// In passthrough mode (USB <-> UART bridge).
    Passthrough,
}

/// Relay controller.
///
/// Generic over the hardware implementations using embedded-hal traits.
pub struct Relay<D, U, S, T>
where
    D: DshotOutput,
    U: Read + Write,
    S: Read + Write,
    T: DelayNs,
{
    dshot: D,
    // Stored for passthrough mode (implemented in firmware, not generic state machine)
    _uart: U,
    _usb: S,
    delay: T,
    state: RelayState,
    dshot_speed: DshotSpeed,
}

impl<D, U, S, T> Relay<D, U, S, T>
where
    D: DshotOutput,
    U: Read + Write,
    S: Read + Write,
    T: DelayNs,
{
    /// Create a new relay controller.
    pub fn new(dshot: D, uart: U, usb: S, delay: T) -> Self {
        Self {
            dshot,
            _uart: uart,
            _usb: usb,
            delay,
            state: RelayState::Idle,
            dshot_speed: DshotSpeed::Dshot300, // Default
        }
    }

    /// Get current state.
    pub fn state(&self) -> RelayState {
        self.state
    }

    /// Handle a parsed command and return a response.
    pub async fn handle_command(&mut self, cmd: Command) -> Response {
        match cmd.op {
            CommandOp::Ping => Response::ok(),

            CommandOp::Info => Response::ok_with_data(b"esc-relay v0.1"),

            CommandOp::EnterBootloader => self.handle_enter_bootloader(&cmd.payload).await,

            CommandOp::SendDshot => self.handle_send_dshot(&cmd.payload).await,

            CommandOp::EnterPassthrough => {
                self.state = RelayState::Passthrough;
                Response::ok()
            }

            CommandOp::ExitPassthrough => {
                self.state = RelayState::Idle;
                Response::ok()
            }

            CommandOp::Reset => {
                self.state = RelayState::Idle;
                Response::ok()
            }
        }
    }

    /// Handle EnterBootloader command.
    ///
    /// Sends the DShot command sequence to put ESC into bootloader mode.
    async fn handle_enter_bootloader(&mut self, payload: &[u8]) -> Response {
        // Parse DShot speed from payload
        if let Some(&speed_byte) = payload.first() {
            if let Some(speed) = DshotSpeed::from_byte(speed_byte) {
                self.dshot_speed = speed;
            } else {
                return Response::error(Status::InvalidPayload);
            }
        }

        // Bootloader entry sequence:
        // 1. Send motor stop (command 0) with telemetry=true, 10x
        // 2. Send motor stop with telemetry=false, 10x
        // 3. Repeat a few times to ensure ESC sees it

        let frame_with_tlm = encode(DshotCommand::MotorStop.as_throttle(), true);
        let frame_no_tlm = encode(DshotCommand::MotorStop.as_throttle(), false);

        // Repeat the sequence 3 times
        for _ in 0..3 {
            // 10 frames with telemetry
            for _ in 0..10 {
                if self.dshot.send_frame(frame_with_tlm).is_err() {
                    return Response::error(Status::Failed);
                }
                self.delay.delay_us(100).await;
            }

            // 10 frames without telemetry
            for _ in 0..10 {
                if self.dshot.send_frame(frame_no_tlm).is_err() {
                    return Response::error(Status::Failed);
                }
                self.delay.delay_us(100).await;
            }
        }

        // Small delay for ESC to enter bootloader
        self.delay.delay_ms(10).await;

        Response::ok()
    }

    /// Handle SendDshot command (raw frame for testing).
    async fn handle_send_dshot(&mut self, payload: &[u8]) -> Response {
        if payload.len() < 3 {
            return Response::error(Status::InvalidPayload);
        }

        let frame = u16::from_be_bytes([payload[0], payload[1]]);
        let count = payload[2] as usize;

        for _ in 0..count {
            if self.dshot.send_frame(frame).is_err() {
                return Response::error(Status::Failed);
            }
            self.delay.delay_us(100).await;
        }

        Response::ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_state_enum() {
        assert_eq!(RelayState::Idle, RelayState::Idle);
        assert_ne!(RelayState::Idle, RelayState::Passthrough);
    }

    #[test]
    fn dshot_speed_parsing() {
        assert_eq!(DshotSpeed::from_byte(0), Some(DshotSpeed::Dshot150));
        assert_eq!(DshotSpeed::from_byte(1), Some(DshotSpeed::Dshot300));
        assert_eq!(DshotSpeed::from_byte(2), Some(DshotSpeed::Dshot600));
        assert_eq!(DshotSpeed::from_byte(3), Some(DshotSpeed::Dshot1200));
        assert_eq!(DshotSpeed::from_byte(4), None);
    }
}
