//! postcard-rpc over USB CDC-ACM, COBS-framed.
//!
//! The same framing Elle uses over RTT (`elle-system/src/rpc.rs`): each message is
//! a postcard-rpc header + postcard body, COBS-encoded, followed by a 0x00
//! delimiter. On the host, postcard-rpc's `cobs-serial` client speaks it.

use core::fmt::Arguments;

use embassy_rp::peripherals::USB;
use embassy_rp::usb::Driver;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_usb::class::cdc_acm::{Receiver, Sender};
use postcard_rpc::header::{VarHeader, VarKeyKind};
use postcard_rpc::server::{WireRx, WireRxErrorKind, WireSpawn, WireTx, WireTxErrorKind};
use serde::Serialize;
use static_cell::StaticCell;

use crate::frame_buf::FrameBuf;

/// Largest frame either way, before COBS.
pub const FRAME_SIZE: usize = 1024;
/// USB full-speed bulk packet size.
pub const PACKET_SIZE: usize = 64;

type UsbDriver = Driver<'static, USB>;

struct TxInner {
    cdc: Sender<'static, UsbDriver>,
    plain: [u8; FRAME_SIZE],
    encoded: [u8; FRAME_SIZE + FRAME_SIZE / 254 + 2],
}

/// Sending half: shared by the server (replies), so behind an async mutex.
#[derive(Clone, Copy)]
pub struct CdcTx {
    inner: &'static Mutex<CriticalSectionRawMutex, TxInner>,
}

impl CdcTx {
    /// Take the CDC sender. Call once.
    pub fn new(cdc: Sender<'static, UsbDriver>) -> Self {
        static INNER: StaticCell<Mutex<CriticalSectionRawMutex, TxInner>> = StaticCell::new();
        let inner = INNER.init(Mutex::new(TxInner {
            cdc,
            plain: [0; FRAME_SIZE],
            encoded: [0; FRAME_SIZE + FRAME_SIZE / 254 + 2],
        }));
        Self { inner }
    }
}

impl TxInner {
    /// COBS-encode `plain[..len]`, append the delimiter and write it out.
    async fn send_plain(&mut self, len: usize) -> Result<(), WireTxErrorKind> {
        let n = cobs::encode(&self.plain[..len], &mut self.encoded);
        self.encoded[n] = 0x00;
        let frame = &self.encoded[..=n];
        for packet in frame.chunks(PACKET_SIZE) {
            self.cdc
                .write_packet(packet)
                .await
                .map_err(|_| WireTxErrorKind::ConnectionClosed)?;
        }
        // A transfer ending on a full packet is only delivered after a short one.
        if frame.len().is_multiple_of(PACKET_SIZE) {
            self.cdc
                .write_packet(&[])
                .await
                .map_err(|_| WireTxErrorKind::ConnectionClosed)?;
        }
        Ok(())
    }
}

impl WireTx for CdcTx {
    type Error = WireTxErrorKind;

    async fn wait_connection(&self) {
        self.inner.lock().await.cdc.wait_connection().await;
    }

    async fn send<T: Serialize + ?Sized>(
        &self,
        hdr: VarHeader,
        msg: &T,
    ) -> Result<(), Self::Error> {
        let mut inner = self.inner.lock().await;
        let Some((hdr_slice, rest)) = hdr.write_to_slice(&mut inner.plain) else {
            return Err(WireTxErrorKind::Other);
        };
        let hdr_len = hdr_slice.len();
        let body_len = postcard::to_slice(msg, rest)
            .map_err(|_| WireTxErrorKind::Other)?
            .len();
        inner.send_plain(hdr_len + body_len).await
    }

    async fn send_raw(&self, buf: &[u8]) -> Result<(), Self::Error> {
        let mut inner = self.inner.lock().await;
        let len = buf.len();
        if len > FRAME_SIZE {
            return Err(WireTxErrorKind::Other);
        }
        inner.plain[..len].copy_from_slice(buf);
        inner.send_plain(len).await
    }

    async fn send_log_str(&self, _kkind: VarKeyKind, _s: &str) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn send_log_fmt<'a>(
        &self,
        _kkind: VarKeyKind,
        _a: Arguments<'a>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// Receiving half.
pub struct CdcRx {
    cdc: Receiver<'static, UsbDriver>,
    frames: FrameBuf<FRAME_SIZE>,
}

impl CdcRx {
    /// Take the CDC receiver.
    pub fn new(cdc: Receiver<'static, UsbDriver>) -> Self {
        Self {
            cdc,
            frames: FrameBuf::new(),
        }
    }
}

impl WireRx for CdcRx {
    type Error = WireRxErrorKind;

    async fn wait_connection(&mut self) {
        self.cdc.wait_connection().await;
    }

    async fn receive<'a>(&mut self, buf: &'a mut [u8]) -> Result<&'a mut [u8], Self::Error> {
        loop {
            // Serve every buffered frame before reading more (see FrameBuf).
            while let Some(frame) = self.frames.pop_frame(buf) {
                if let Ok(len) = frame
                    && len > 0
                    && let Ok(decoded) = cobs::decode_in_place(&mut buf[..len])
                {
                    return Ok(&mut buf[..decoded]);
                }
            }

            // No room for a whole USB packet without a delimiter in sight: out of
            // sync or an over-long frame. Resynchronise.
            if self.frames.is_stuck() || self.frames.spare().len() < PACKET_SIZE {
                self.frames.clear();
            }

            let n = self
                .cdc
                .read_packet(&mut self.frames.spare()[..PACKET_SIZE])
                .await
                .map_err(|_| WireRxErrorKind::ConnectionClosed)?;
            self.frames.commit(n);
        }
    }
}

/// No handler is spawned; `define_dispatch!` still needs a spawner type.
#[derive(Clone)]
pub struct NoSpawn;

impl WireSpawn for NoSpawn {
    type Error = core::convert::Infallible;
    type Info = ();

    fn info(&self) -> &Self::Info {
        &()
    }
}
