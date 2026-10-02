//! Relay state machine.
//!
//! The relay operates in two states:
//! 1. **Idle**: Waiting for commands from CLI
//! 2. **Passthrough**: Transparent bridge between USB and UART

use crate::protocol::{Command, CommandOp, Response};
use crate::traits::{Read, Write};

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
pub struct Relay<U, S>
where
    U: Read + Write,
    S: Read + Write,
{
    // Stored for passthrough mode (implemented in firmware, not generic state machine)
    _uart: U,
    _usb: S,
    state: RelayState,
}

impl<U, S> Relay<U, S>
where
    U: Read + Write,
    S: Read + Write,
{
    /// Create a new relay controller.
    pub const fn new(uart: U, usb: S) -> Self {
        Self {
            _uart: uart,
            _usb: usb,
            state: RelayState::Idle,
        }
    }

    /// Get current state.
    pub const fn state(&self) -> RelayState {
        self.state
    }

    /// Handle a parsed command and return a response.
    pub fn handle_command(&mut self, cmd: Command) -> Response {
        match cmd.op {
            CommandOp::Ping => Response::ok(),

            CommandOp::Info => Response::ok_with_data(b"esc-relay v0.1"),

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_state_enum() {
        assert_eq!(RelayState::Idle, RelayState::Idle);
        assert_ne!(RelayState::Idle, RelayState::Passthrough);
    }
}
