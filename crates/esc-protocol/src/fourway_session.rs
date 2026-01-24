//! Four-Way Interface session state machine.
//!
//! Manages the full connection flow through a Betaflight flight controller:
//!
//! 1. MSP handshake with FC (verify API version)
//! 2. Enter Four-Way passthrough mode
//! 3. Connect to individual ESC
//! 4. Read/Write operations
//! 5. Disconnect and reset
//!
//! # Example
//!
//! ```ignore
//! use esc_protocol::{FourWaySession, FourWayState};
//!
//! let mut session = FourWaySession::new();
//!
//! // Step 1: Verify FC connection
//! let packet = session.api_version_command()?;
//! serial.write(&packet)?;
//! let response = serial.read_response()?;
//! session.handle_api_version_response(&response)?;
//!
//! // Step 2: Enter passthrough
//! let packet = session.enter_passthrough_command()?;
//! serial.write(&packet)?;
//! let response = serial.read_response()?;
//! let motor_count = session.handle_passthrough_response(&response)?;
//!
//! // Step 3: Connect to ESC 0
//! let packet = session.connect_esc_command(0)?;
//! serial.write(&packet)?;
//! let response = serial.read_response()?;
//! session.handle_connect_esc_response(0, &response)?;
//!
//! // Step 4: Read config from ESC
//! let packet = session.read_eeprom_command(0, 0x7C00)?;
//! // ... etc
//! ```

use crate::fourway::{
    build_device_init_flash, build_device_page_erase, build_device_read,
    build_device_read_eeprom, build_device_reset, build_device_write,
    build_device_write_eeprom, build_interface_exit, parse_fourway_response,
    parse_init_flash_response, FourWayError, FourWayMcuType,
};
use crate::msp::{
    build_api_version_packet, build_reboot_packet, build_set_4way_if_packet,
    parse_4way_if_response, parse_msp_response, MspError,
};

/// Four-Way session state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FourWayState {
    /// Not connected to FC
    #[default]
    Disconnected,
    /// Connected to FC, not yet in 4-way mode
    FcConnected,
    /// In 4-way passthrough mode
    InPassthrough {
        /// Number of motors reported by FC
        motor_count: u8,
    },
    /// Connected to specific ESC
    EscConnected {
        /// 0-based motor index
        motor_index: u8,
        /// MCU type (if detected)
        mcu_type: Option<FourWayMcuType>,
    },
}

/// Four-Way session manager.
///
/// This is a state machine that tracks the connection flow and generates
/// appropriate packets for each step. The caller is responsible for
/// actually sending/receiving data over serial.
#[derive(Debug, Default)]
pub struct FourWaySession {
    state: FourWayState,
}

impl FourWaySession {
    /// Create new disconnected session.
    pub const fn new() -> Self {
        Self {
            state: FourWayState::Disconnected,
        }
    }

    /// Get current state.
    pub const fn state(&self) -> FourWayState {
        self.state
    }

    /// Check if in passthrough mode (either InPassthrough or EscConnected).
    pub const fn is_in_passthrough(&self) -> bool {
        matches!(
            self.state,
            FourWayState::InPassthrough { .. } | FourWayState::EscConnected { .. }
        )
    }

    /// Check if connected to an ESC.
    pub const fn is_esc_connected(&self) -> bool {
        matches!(self.state, FourWayState::EscConnected { .. })
    }

    /// Get connected motor index (if connected).
    pub const fn motor_index(&self) -> Option<u8> {
        match self.state {
            FourWayState::EscConnected { motor_index, .. } => Some(motor_index),
            _ => None,
        }
    }

    /// Get MCU type (if connected and known).
    pub const fn mcu_type(&self) -> Option<FourWayMcuType> {
        match self.state {
            FourWayState::EscConnected { mcu_type, .. } => mcu_type,
            _ => None,
        }
    }

    /// Get motor count (if in passthrough or ESC connected).
    pub const fn motor_count(&self) -> Option<u8> {
        match self.state {
            FourWayState::InPassthrough { motor_count } => Some(motor_count),
            // When ESC connected, we don't track motor_count, but it was at least 1
            FourWayState::EscConnected { .. } => Some(8), // Assume max
            _ => None,
        }
    }

    /// Force reset to disconnected state.
    ///
    /// Use this to recover from errors or start fresh.
    pub fn reset(&mut self) {
        self.state = FourWayState::Disconnected;
    }

    // ========================================================================
    // Command Generation (returns packets to send)
    // ========================================================================

    /// Get MSP_API_VERSION packet.
    ///
    /// Call when state == Disconnected.
    pub fn api_version_command(&self) -> Result<[u8; 6], FourWaySessionError> {
        match self.state {
            FourWayState::Disconnected => Ok(build_api_version_packet()),
            _ => Err(FourWaySessionError::InvalidState {
                expected: "Disconnected",
                actual: self.state,
            }),
        }
    }

    /// Get MSP_SET_4WAY_IF packet.
    ///
    /// Call when state == FcConnected.
    pub fn enter_passthrough_command(&self) -> Result<[u8; 6], FourWaySessionError> {
        match self.state {
            FourWayState::FcConnected => Ok(build_set_4way_if_packet()),
            _ => Err(FourWaySessionError::InvalidState {
                expected: "FcConnected",
                actual: self.state,
            }),
        }
    }

    /// Get DeviceInitFlash packet.
    ///
    /// Call when state == InPassthrough.
    pub fn connect_esc_command(&self, motor_index: u8) -> Result<[u8; 6], FourWaySessionError> {
        match self.state {
            FourWayState::InPassthrough { motor_count } => {
                if motor_index >= motor_count {
                    return Err(FourWaySessionError::InvalidMotorIndex {
                        index: motor_index,
                        count: motor_count,
                    });
                }
                Ok(build_device_init_flash(motor_index))
            }
            _ => Err(FourWaySessionError::NotInPassthrough),
        }
    }

    /// Get DeviceReset packet.
    ///
    /// Call when state == EscConnected.
    pub fn reset_esc_command(&self) -> Result<[u8; 6], FourWaySessionError> {
        match self.state {
            FourWayState::EscConnected { motor_index, .. } => Ok(build_device_reset(motor_index)),
            _ => Err(FourWaySessionError::EscNotConnected),
        }
    }

    /// Get DeviceRead packet.
    ///
    /// - `size`: Number of bytes to read (0 = 256)
    /// - `address`: Flash address (will be adjusted for G071 automatically)
    pub fn read_command(&self, size: u8, address: u16) -> Result<[u8; 6], FourWaySessionError> {
        match self.state {
            FourWayState::EscConnected { mcu_type, .. } => {
                let adjusted_addr = match mcu_type {
                    Some(mcu) => mcu.adjust_address(address),
                    None => address,
                };
                Ok(build_device_read(size, adjusted_addr))
            }
            _ => Err(FourWaySessionError::EscNotConnected),
        }
    }

    /// Get DeviceReadEeprom packet.
    ///
    /// - `size`: Number of bytes to read (0 = 256)
    /// - `address`: EEPROM address (will be adjusted for G071 automatically)
    pub fn read_eeprom_command(
        &self,
        size: u8,
        address: u16,
    ) -> Result<[u8; 6], FourWaySessionError> {
        match self.state {
            FourWayState::EscConnected { mcu_type, .. } => {
                let adjusted_addr = match mcu_type {
                    Some(mcu) => mcu.adjust_address(address),
                    None => address,
                };
                Ok(build_device_read_eeprom(size, adjusted_addr))
            }
            _ => Err(FourWaySessionError::EscNotConnected),
        }
    }

    /// Build DeviceWrite packet into provided buffer.
    ///
    /// - `address`: Flash address (will be adjusted for G071 automatically)
    /// - `data`: Data to write
    /// - `buffer`: Output buffer (must be at least 6 + data.len() bytes)
    ///
    /// Returns the packet length.
    pub fn write_command(
        &self,
        address: u16,
        data: &[u8],
        buffer: &mut [u8],
    ) -> Result<usize, FourWaySessionError> {
        match self.state {
            FourWayState::EscConnected { mcu_type, .. } => {
                let adjusted_addr = match mcu_type {
                    Some(mcu) => mcu.adjust_address(address),
                    None => address,
                };
                Ok(build_device_write(adjusted_addr, data, buffer))
            }
            _ => Err(FourWaySessionError::EscNotConnected),
        }
    }

    /// Build DeviceWriteEeprom packet into provided buffer.
    ///
    /// - `address`: EEPROM address (will be adjusted for G071 automatically)
    /// - `data`: Data to write
    /// - `buffer`: Output buffer (must be at least 6 + data.len() bytes)
    ///
    /// Returns the packet length.
    pub fn write_eeprom_command(
        &self,
        address: u16,
        data: &[u8],
        buffer: &mut [u8],
    ) -> Result<usize, FourWaySessionError> {
        match self.state {
            FourWayState::EscConnected { mcu_type, .. } => {
                let adjusted_addr = match mcu_type {
                    Some(mcu) => mcu.adjust_address(address),
                    None => address,
                };
                Ok(build_device_write_eeprom(adjusted_addr, data, buffer))
            }
            _ => Err(FourWaySessionError::EscNotConnected),
        }
    }

    /// Get DevicePageErase packet.
    ///
    /// - `address`: Address of the page to erase (will be adjusted for G071)
    pub fn erase_page_command(&self, address: u16) -> Result<[u8; 6], FourWaySessionError> {
        match self.state {
            FourWayState::EscConnected { mcu_type, .. } => {
                let adjusted_addr = match mcu_type {
                    Some(mcu) => mcu.adjust_address(address),
                    None => address,
                };
                Ok(build_device_page_erase(adjusted_addr))
            }
            _ => Err(FourWaySessionError::EscNotConnected),
        }
    }

    /// Get InterfaceExit packet.
    ///
    /// Call when in passthrough mode to exit back to FC connected state.
    pub fn exit_passthrough_command(&self) -> Result<[u8; 6], FourWaySessionError> {
        if self.is_in_passthrough() {
            Ok(build_interface_exit())
        } else {
            Err(FourWaySessionError::NotInPassthrough)
        }
    }

    /// Get MSP_SET_REBOOT packet.
    ///
    /// **Important:** Always reboot the FC after exiting 4-way mode,
    /// otherwise it may hold the last throttle value!
    pub fn reboot_fc_command(&self) -> [u8; 6] {
        // Can reboot anytime - no state check needed
        build_reboot_packet()
    }

    // ========================================================================
    // Response Handling (updates state)
    // ========================================================================

    /// Handle MSP_API_VERSION response.
    ///
    /// On success, transitions to FcConnected.
    pub fn handle_api_version_response(&mut self, data: &[u8]) -> Result<(), FourWaySessionError> {
        // Verify we got a valid MSP response
        let _ = parse_msp_response(data).map_err(FourWaySessionError::Msp)?;

        match self.state {
            FourWayState::Disconnected => {
                self.state = FourWayState::FcConnected;
                Ok(())
            }
            _ => Err(FourWaySessionError::InvalidState {
                expected: "Disconnected",
                actual: self.state,
            }),
        }
    }

    /// Handle MSP_SET_4WAY_IF response.
    ///
    /// On success, transitions to InPassthrough with motor count.
    /// Returns the motor count.
    pub fn handle_passthrough_response(&mut self, data: &[u8]) -> Result<u8, FourWaySessionError> {
        let motor_count = parse_4way_if_response(data).map_err(FourWaySessionError::Msp)?;

        match self.state {
            FourWayState::FcConnected => {
                self.state = FourWayState::InPassthrough { motor_count };
                Ok(motor_count)
            }
            _ => Err(FourWaySessionError::InvalidState {
                expected: "FcConnected",
                actual: self.state,
            }),
        }
    }

    /// Handle DeviceInitFlash response.
    ///
    /// On success, transitions to EscConnected with MCU info.
    /// Returns the MCU type if detected.
    pub fn handle_connect_esc_response(
        &mut self,
        motor_index: u8,
        data: &[u8],
    ) -> Result<Option<FourWayMcuType>, FourWaySessionError> {
        match self.state {
            FourWayState::InPassthrough { .. } => {
                let info =
                    parse_init_flash_response(data).map_err(FourWaySessionError::FourWay)?;

                self.state = FourWayState::EscConnected {
                    motor_index,
                    mcu_type: info.mcu_type,
                };

                Ok(info.mcu_type)
            }
            _ => Err(FourWaySessionError::NotInPassthrough),
        }
    }

    /// Handle DeviceReset response.
    ///
    /// On success, transitions back to InPassthrough.
    pub fn handle_reset_esc_response(&mut self, data: &[u8]) -> Result<(), FourWaySessionError> {
        let response = parse_fourway_response(data).map_err(FourWaySessionError::FourWay)?;

        if !response.ack.is_ok() {
            return Err(FourWaySessionError::FourWay(FourWayError::CommandFailed {
                ack: response.ack,
            }));
        }

        match self.state {
            FourWayState::EscConnected { .. } => {
                // Return to passthrough - we don't know the exact motor count anymore
                self.state = FourWayState::InPassthrough { motor_count: 8 };
                Ok(())
            }
            _ => Err(FourWaySessionError::EscNotConnected),
        }
    }

    /// Handle a generic Four-Way response (for read/write operations).
    ///
    /// Returns the parsed response for further processing.
    /// Does not change state.
    pub fn handle_fourway_response<'a>(
        &self,
        data: &'a [u8],
    ) -> Result<crate::fourway::FourWayResponse<'a>, FourWaySessionError> {
        if !self.is_esc_connected() {
            return Err(FourWaySessionError::EscNotConnected);
        }

        let response = parse_fourway_response(data).map_err(FourWaySessionError::FourWay)?;

        if !response.ack.is_ok() {
            return Err(FourWaySessionError::FourWay(FourWayError::CommandFailed {
                ack: response.ack,
            }));
        }

        Ok(response)
    }

    /// Handle InterfaceExit response.
    ///
    /// Transitions to FcConnected.
    pub fn handle_exit_response(&mut self, data: &[u8]) -> Result<(), FourWaySessionError> {
        // Exit response is a Four-Way response
        let response = parse_fourway_response(data).map_err(FourWaySessionError::FourWay)?;

        if !response.ack.is_ok() {
            return Err(FourWaySessionError::FourWay(FourWayError::CommandFailed {
                ack: response.ack,
            }));
        }

        if self.is_in_passthrough() {
            self.state = FourWayState::FcConnected;
            Ok(())
        } else {
            Err(FourWaySessionError::NotInPassthrough)
        }
    }
}

/// Four-Way session errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FourWaySessionError {
    /// Wrong state for operation
    InvalidState {
        expected: &'static str,
        actual: FourWayState,
    },
    /// ESC not connected
    EscNotConnected,
    /// Not in passthrough mode
    NotInPassthrough,
    /// Invalid motor index
    InvalidMotorIndex { index: u8, count: u8 },
    /// MSP protocol error
    Msp(MspError),
    /// Four-Way protocol error
    FourWay(FourWayError),
}

impl core::fmt::Display for FourWaySessionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidState { expected, actual } => {
                write!(f, "invalid state: expected {}, got {:?}", expected, actual)
            }
            Self::EscNotConnected => write!(f, "ESC not connected"),
            Self::NotInPassthrough => write!(f, "not in passthrough mode"),
            Self::InvalidMotorIndex { index, count } => {
                write!(f, "invalid motor index {} (max {})", index, count - 1)
            }
            Self::Msp(e) => write!(f, "MSP error: {}", e),
            Self::FourWay(e) => write!(f, "Four-Way error: {}", e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_state() {
        let session = FourWaySession::new();
        assert_eq!(session.state(), FourWayState::Disconnected);
        assert!(!session.is_in_passthrough());
        assert!(!session.is_esc_connected());
    }

    #[test]
    fn api_version_only_when_disconnected() {
        let session = FourWaySession::new();
        assert!(session.api_version_command().is_ok());

        let mut session = FourWaySession::new();
        session.state = FourWayState::FcConnected;
        assert!(session.api_version_command().is_err());
    }

    #[test]
    fn enter_passthrough_only_when_fc_connected() {
        let mut session = FourWaySession::new();
        assert!(session.enter_passthrough_command().is_err());

        session.state = FourWayState::FcConnected;
        assert!(session.enter_passthrough_command().is_ok());
    }

    #[test]
    fn connect_esc_validates_motor_index() {
        let mut session = FourWaySession::new();
        session.state = FourWayState::InPassthrough { motor_count: 4 };

        assert!(session.connect_esc_command(0).is_ok());
        assert!(session.connect_esc_command(3).is_ok());
        assert!(session.connect_esc_command(4).is_err()); // Out of range
    }

    #[test]
    fn read_write_only_when_esc_connected() {
        let mut session = FourWaySession::new();
        assert!(session.read_command(48, 0x7C00).is_err());

        session.state = FourWayState::EscConnected {
            motor_index: 0,
            mcu_type: Some(FourWayMcuType::Stm32F051),
        };
        assert!(session.read_command(48, 0x7C00).is_ok());
    }

    #[test]
    fn address_adjustment_for_g071() {
        let mut session = FourWaySession::new();
        session.state = FourWayState::EscConnected {
            motor_index: 0,
            mcu_type: Some(FourWayMcuType::Stm32G071),
        };

        let packet = session.read_eeprom_command(48, 0x7E00).unwrap();
        // Address should be divided by 4: 0x7E00 / 4 = 0x1F80
        assert_eq!(packet[2], 0x1F); // High byte
        assert_eq!(packet[3], 0x80); // Low byte
    }

    #[test]
    fn address_no_adjustment_for_f051() {
        let mut session = FourWaySession::new();
        session.state = FourWayState::EscConnected {
            motor_index: 0,
            mcu_type: Some(FourWayMcuType::Stm32F051),
        };

        let packet = session.read_eeprom_command(48, 0x7C00).unwrap();
        // Address should not be adjusted
        assert_eq!(packet[2], 0x7C); // High byte
        assert_eq!(packet[3], 0x00); // Low byte
    }

    #[test]
    fn state_transitions() {
        let mut session = FourWaySession::new();

        // Disconnected -> FcConnected
        // Simulate valid API version response
        let api_response = [0x24, 0x4D, 0x3E, 0x03, 0x68, 0x00, 0x01, 0x2D, 0x03 ^ 0x68 ^ 0x00 ^ 0x01 ^ 0x2D];
        session.handle_api_version_response(&api_response).unwrap();
        assert_eq!(session.state(), FourWayState::FcConnected);

        // FcConnected -> InPassthrough
        let passthrough_response = [0x24, 0x4D, 0x3E, 0x01, 0xF5, 0x04, 0x01 ^ 0xF5 ^ 0x04];
        let motor_count = session.handle_passthrough_response(&passthrough_response).unwrap();
        assert_eq!(motor_count, 4);
        assert_eq!(session.state(), FourWayState::InPassthrough { motor_count: 4 });
    }

    #[test]
    fn reset_clears_state() {
        let mut session = FourWaySession::new();
        session.state = FourWayState::EscConnected {
            motor_index: 2,
            mcu_type: Some(FourWayMcuType::Stm32G071),
        };

        session.reset();
        assert_eq!(session.state(), FourWayState::Disconnected);
    }

    #[test]
    fn reboot_available_anytime() {
        let session = FourWaySession::new();
        // Should not panic or error
        let _ = session.reboot_fc_command();
    }
}
