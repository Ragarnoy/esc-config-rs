//! ICD endpoint handlers: each request runs one whole ESC operation.
//!
//! The dispatch context owns the one-wire UART and, while an ESC is connected, its
//! connect reply and layout. Each request resumes an `esc_session::Esc` over the
//! borrowed UART, so the bootloader sequencing never crosses USB.

use embassy_rp::peripherals::PIO0;
use esc_config_icd::{
    AckResp, Block, BlockResp, ChunkReq, ConnectEndpoint, ConnectResp, DisconnectEndpoint,
    EEPROM_SIZE, ENDPOINT_LIST, Eeprom, EepromResp, EscError, EscInfo, FlashBeginEndpoint,
    FlashChunkEndpoint, FlashFinishEndpoint, MAX_BLOCK, ReadEepromEndpoint, ReadEndpoint,
    ReadFileNameEndpoint, ReadFirmwareEndpoint, ReadFirmwareReq, ReadReq, RunEndpoint, ServerInfo,
    ServerInfoEndpoint, TOPICS_IN_LIST, TOPICS_OUT_LIST, WriteEepromEndpoint,
};
use esc_onewire_rp::OneWireUart;
use esc_protocol::layout::Layout;
use esc_protocol::packet::ConnectResponse;
use esc_session::{Esc, Timing};
use postcard_rpc::header::VarHeader;

use crate::wire::{CdcTx, NoSpawn};

/// The relay's UART: PIO0, TX on SM0, RX on SM1.
pub type Uart = OneWireUart<'static, PIO0, 0, 1>;

/// Dispatch context.
pub struct Context {
    /// One-wire link to the ESC.
    pub uart: Uart,
    /// Connect reply and layout of the connected ESC.
    pub connected: Option<(ConnectResponse, Layout)>,
    /// A flash is in progress and has not failed: `flash/begin` succeeded and so
    /// did every chunk since. The relay enforces the order itself, so no client
    /// can set the boot byte over a half-written app.
    pub flashing: bool,
    /// A flash began on the ESC with this connect reply and has not finished.
    /// Its boot byte is cleared on purpose, so `eeprom/write` refuses to set it:
    /// a client writing back settings it read before the flash would otherwise
    /// let the bootloader start a half-written app. Kept across reconnects of
    /// the same ESC; cleared by a finished flash or a different ESC.
    pub flash_pending: Option<ConnectResponse>,
}

impl Context {
    /// The connected ESC, over the borrowed UART.
    fn esc(&mut self) -> Result<Esc<&mut Uart>, EscError> {
        let (reply, layout) = self.connected.ok_or(EscError::NotConnected)?;
        Ok(Esc::resume(
            &mut self.uart,
            Timing::default(),
            reply,
            layout,
        ))
    }
}

/// EEPROM byte 0 value that lets the bootloader start the app.
const BOOT_ENABLED: u8 = 0x01;

/// Parse a decimal version component at compile time (from Elle's `rpc_app.rs`).
const fn version_component(s: &str) -> u8 {
    let bytes = s.as_bytes();
    assert!(!bytes.is_empty(), "empty version component");
    let mut value: u32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        let digit = bytes[i];
        assert!(digit.is_ascii_digit(), "non-numeric version component");
        value = value * 10 + (digit - b'0') as u32;
        assert!(value <= u8::MAX as u32, "version component over 255");
        i += 1;
    }
    value as u8
}

const SERVER_INFO: ServerInfo = ServerInfo {
    major: version_component(env!("CARGO_PKG_VERSION_MAJOR")),
    minor: version_component(env!("CARGO_PKG_VERSION_MINOR")),
    patch: version_component(env!("CARGO_PKG_VERSION_PATCH")),
};

fn block(data: &[u8]) -> Block {
    // Callers pass at most MAX_BLOCK bytes.
    Block::from_slice(data).unwrap_or_default()
}

fn check_len(len: u16) -> Result<usize, EscError> {
    match usize::from(len) {
        n @ 1..=MAX_BLOCK => Ok(n),
        _ => Err(EscError::InvalidLength(len)),
    }
}

fn server_info(_ctx: &mut Context, _hdr: VarHeader, _req: ()) -> ServerInfo {
    SERVER_INFO
}

async fn connect(ctx: &mut Context, _hdr: VarHeader, _req: ()) -> ConnectResp {
    ctx.connected = None;
    ctx.flashing = false;
    let esc = Esc::connect_with(&mut ctx.uart, Timing::default()).await?;
    let info = EscInfo::of(&esc);
    let parts = (*esc.connect_reply(), *esc.layout());
    if ctx.flash_pending.is_some_and(|r| r != parts.0) {
        ctx.flash_pending = None;
    }
    ctx.connected = Some(parts);
    defmt::info!("ESC connected: 0x{:04X}", info.device_id);
    Ok(info)
}

fn disconnect(ctx: &mut Context, _hdr: VarHeader, _req: ()) -> AckResp {
    ctx.connected = None;
    ctx.flashing = false;
    Ok(())
}

async fn run(ctx: &mut Context, _hdr: VarHeader, _req: ()) -> AckResp {
    let esc = ctx.esc()?;
    let result = esc.run().await.map(|_| ()).map_err(EscError::from);
    ctx.connected = None;
    ctx.flashing = false;
    result
}

async fn read_eeprom(ctx: &mut Context, _hdr: VarHeader, _req: ()) -> EepromResp {
    let data = ctx.esc()?.read_eeprom().await?;
    Ok(Eeprom::from_slice(&data).unwrap_or_default())
}

async fn write_eeprom(ctx: &mut Context, _hdr: VarHeader, req: Eeprom) -> AckResp {
    let data: [u8; EEPROM_SIZE] = req
        .as_slice()
        .try_into()
        .map_err(|_| EscError::InvalidLength(req.len() as u16))?;
    if ctx.flash_pending.is_some() && data[0] == BOOT_ENABLED {
        return Err(EscError::FlashSequence);
    }
    ctx.esc()?.write_eeprom(&data).await?;
    Ok(())
}

async fn read_file_name(ctx: &mut Context, _hdr: VarHeader, _req: ()) -> BlockResp {
    let name = ctx.esc()?.read_file_name().await?;
    Ok(block(&name))
}

async fn read(ctx: &mut Context, _hdr: VarHeader, req: ReadReq) -> BlockResp {
    let len = check_len(req.len)?;
    let mut buf = [0u8; MAX_BLOCK];
    ctx.esc()?.read_at(req.address, &mut buf[..len]).await?;
    Ok(block(&buf[..len]))
}

async fn flash_begin(ctx: &mut Context, _hdr: VarHeader, _req: ()) -> AckResp {
    let mut esc = ctx.esc()?;
    let reply = *esc.connect_reply();
    let result = esc.begin_flash().await;
    // Pending even if begin failed: the boot byte may already be cleared.
    ctx.flash_pending = Some(reply);
    ctx.flashing = result.is_ok();
    Ok(result?)
}

async fn flash_chunk(ctx: &mut Context, _hdr: VarHeader, req: ChunkReq) -> AckResp {
    if !ctx.flashing {
        return Err(EscError::FlashSequence);
    }
    let result = ctx
        .esc()?
        .write_firmware_chunk(req.offset as usize, &req.data)
        .await;
    ctx.flashing = result.is_ok();
    Ok(result?)
}

async fn flash_finish(ctx: &mut Context, _hdr: VarHeader, _req: ()) -> AckResp {
    if !ctx.flashing {
        return Err(EscError::FlashSequence);
    }
    ctx.esc()?.finish_flash().await?;
    ctx.flashing = false;
    ctx.flash_pending = None;
    Ok(())
}

async fn read_firmware(ctx: &mut Context, _hdr: VarHeader, req: ReadFirmwareReq) -> BlockResp {
    let len = check_len(req.len)?;
    let mut buf = [0u8; MAX_BLOCK];
    ctx.esc()?
        .read_firmware(req.offset as usize, &mut buf[..len])
        .await?;
    Ok(block(&buf[..len]))
}

postcard_rpc::define_dispatch! {
    app: RelayApp;

    spawn_fn: no_spawn;
    tx_impl: CdcTx;
    spawn_impl: NoSpawn;
    context: Context;

    endpoints: {
        list: ENDPOINT_LIST;

        | EndpointTy            | kind      | handler        |
        | ----------            | ----      | -------        |
        | ServerInfoEndpoint    | blocking  | server_info    |
        | ConnectEndpoint       | async     | connect        |
        | DisconnectEndpoint    | blocking  | disconnect     |
        | RunEndpoint           | async     | run            |
        | ReadEepromEndpoint    | async     | read_eeprom    |
        | WriteEepromEndpoint   | async     | write_eeprom   |
        | ReadFileNameEndpoint  | async     | read_file_name |
        | ReadEndpoint          | async     | read           |
        | FlashBeginEndpoint    | async     | flash_begin    |
        | FlashChunkEndpoint    | async     | flash_chunk    |
        | FlashFinishEndpoint   | async     | flash_finish   |
        | ReadFirmwareEndpoint  | async     | read_firmware  |
    };
    topics_in: {
        list: TOPICS_IN_LIST;

        | TopicTy | kind | handler |
        | ------- | ---- | ------- |
    };
    topics_out: {
        list: TOPICS_OUT_LIST;
    };
}
