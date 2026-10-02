//! Hardware abstraction traits.
//!
//! Re-exports standard embedded traits for UART and USB serial I/O.

// Re-export standard traits for convenience
pub use embedded_hal_async::delay::DelayNs;
pub use embedded_io_async::{Read, Write};
