//! Hardware abstraction traits.
//!
//! This module re-exports standard embedded traits and defines
//! domain-specific traits for DShot output.
//!
//! # Standard Traits Used
//!
//! - `embedded_io_async::{Read, Write}` - For UART and USB serial
//! - `embedded_hal_async::delay::DelayNs` - For timing delays
//!
//! # Custom Traits
//!
//! - `DshotOutput` - DShot frame transmission (domain-specific)

// Re-export standard traits for convenience
pub use embedded_hal_async::delay::DelayNs;
pub use embedded_io_async::{Read, Write};

/// DShot output interface.
///
/// Implementations handle the precise timing requirements of DShot
/// (e.g., using PIO on RP2040/RP2350, or DMA+Timer on STM32).
///
/// This is a blocking trait since DShot frame transmission is fast
/// (~27µs for DShot600) and typically done via hardware (PIO/DMA).
pub trait DshotOutput {
    /// Error type for DShot operations.
    type Error;

    /// Send a single DShot frame.
    ///
    /// The frame is a 16-bit value containing throttle/command + telemetry + CRC.
    /// Use `esc_dshot::encode()` to create frames.
    fn send_frame(&mut self, frame: u16) -> Result<(), Self::Error>;

    /// Send multiple identical frames (required for commands).
    ///
    /// DShot commands need to be sent 6+ times to be recognized.
    /// Default implementation calls `send_frame` in a loop.
    fn send_frames(&mut self, frame: u16, count: usize) -> Result<(), Self::Error> {
        for _ in 0..count {
            self.send_frame(frame)?;
        }
        Ok(())
    }
}

/// Async version of DShot output.
///
/// Use this when the DShot transmission is async (e.g., waiting for DMA completion).
pub trait DshotOutputAsync {
    /// Error type for DShot operations.
    type Error;

    /// Send a single DShot frame asynchronously.
    async fn send_frame(&mut self, frame: u16) -> Result<(), Self::Error>;

    /// Send multiple identical frames asynchronously.
    async fn send_frames(&mut self, frame: u16, count: usize) -> Result<(), Self::Error> {
        for _ in 0..count {
            self.send_frame(frame).await?;
        }
        Ok(())
    }
}
