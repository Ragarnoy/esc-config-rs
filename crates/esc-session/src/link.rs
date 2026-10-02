//! The transport the session drives: one half-duplex wire to the ESC.

/// A span of time, in microseconds.
///
/// A local type so the session stays independent of any executor or time driver;
/// convert at the [`Link`] boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Duration {
    micros: u32,
}

impl Duration {
    /// Zero.
    pub const ZERO: Self = Self { micros: 0 };

    /// From microseconds.
    pub const fn from_micros(micros: u32) -> Self {
        Self { micros }
    }

    /// From milliseconds (saturating).
    pub const fn from_millis(millis: u32) -> Self {
        Self {
            micros: millis.saturating_mul(1000),
        }
    }

    /// Microseconds.
    pub const fn as_micros(self) -> u32 {
        self.micros
    }

    /// Milliseconds, rounded up.
    pub const fn as_millis_ceil(self) -> u32 {
        self.micros.div_ceil(1000)
    }

    /// `self + other`, saturating.
    pub const fn saturating_add(self, other: Self) -> Self {
        Self {
            micros: self.micros.saturating_add(other.micros),
        }
    }

    /// `self * n`, saturating.
    pub const fn saturating_mul(self, n: u32) -> Self {
        Self {
            micros: self.micros.saturating_mul(n),
        }
    }
}

/// A half-duplex link to an AM32 bootloader: a one-wire UART at 19200 8N1, or
/// something that forwards to one.
///
/// The bootloader ends a frame when the line has been idle for about 230 µs, so
/// [`Link::send`] must put a whole frame on the wire with no gaps, and must not
/// return before the line is released for the reply.
#[allow(async_fn_in_trait)]
pub trait Link {
    /// Transport error. Timeouts are not errors: see [`Link::recv`].
    type Error;

    /// Send one frame, bytes back to back, then release the line for the reply.
    async fn send(&mut self, frame: &[u8]) -> Result<(), Self::Error>;

    /// Receive up to `buf.len()` bytes, waiting at most `timeout` in total.
    ///
    /// Returns how many bytes arrived. Fewer than `buf.len()` means the timeout ran
    /// out; that is not an error at this level.
    async fn recv(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize, Self::Error>;

    /// Wait, keeping the line idle (high).
    async fn delay(&mut self, duration: Duration);
}
