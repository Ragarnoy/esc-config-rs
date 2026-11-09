//! The relay as an [`EscDevice`]: postcard-rpc over USB CDC, COBS-framed.
//!
//! The relay firmware runs the bootloader session itself; each call here is one
//! request carrying one whole operation (`esc-config-icd`).

use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use esc_config_icd::{
    ChunkReq, ConnectEndpoint, EEPROM_SIZE, EscError, EscInfo, FlashBeginEndpoint,
    FlashChunkEndpoint, FlashFinishEndpoint, RELAY_USB_VID_PID, ReadEepromEndpoint, ReadEndpoint,
    ReadFileNameEndpoint, ReadFirmwareEndpoint, ReadFirmwareReq, ReadReq, RunEndpoint,
    ServerInfoEndpoint, WriteEepromEndpoint,
};
use postcard_rpc::Endpoint;
use postcard_rpc::header::VarSeqKind;
use postcard_rpc::host_client::HostClient;
use postcard_rpc::standard_icd::{ERROR_PATH, WireError};
use postcard_schema::Schema;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::device::{EscDevice, Session};

/// Longest any single operation may take (a 256-byte flash chunk with retries is
/// well under a second; connect with retries about one).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// The relay, over its USB CDC port.
pub struct RelayClient {
    runtime: tokio::runtime::Runtime,
    client: HostClient<WireError>,
}

impl RelayClient {
    /// Open the relay's serial port and check that relay firmware answers.
    pub fn open(port: &str) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("failed to start the async runtime")?;
        let client = runtime
            .block_on(async {
                HostClient::<WireError>::try_new_serial_cobs(
                    port,
                    ERROR_PATH,
                    8,
                    115_200,
                    VarSeqKind::Seq2,
                )
            })
            .map_err(|e| anyhow!("failed to open {port}: {e}"))?;
        let mut relay = Self { runtime, client };
        relay
            .call::<ServerInfoEndpoint>(&())
            .context("no answer from the relay firmware (old firmware, or not a relay?)")?;
        Ok(relay)
    }

    fn call<E>(&mut self, req: &E::Request) -> Result<E::Response>
    where
        E: Endpoint,
        E::Request: Serialize + Schema,
        E::Response: DeserializeOwned + Schema,
    {
        let client = &self.client;
        self.runtime.block_on(async {
            match tokio::time::timeout(REQUEST_TIMEOUT, client.send_resp::<E>(req)).await {
                Ok(Ok(resp)) => Ok(resp),
                Ok(Err(e)) => Err(anyhow!("relay request {} failed: {e:?}", E::PATH)),
                Err(_) => Err(anyhow!("relay request {} timed out", E::PATH)),
            }
        })
    }

    /// Call an endpoint whose response is `Result<T, EscError>`.
    fn op<E, T>(&mut self, req: &E::Request) -> Result<T>
    where
        E: Endpoint<Response = Result<T, EscError>>,
        E::Request: Serialize + Schema,
        T: DeserializeOwned + Schema,
    {
        Ok(self.call::<E>(req)??)
    }
}

impl EscDevice for RelayClient {
    fn connect(&mut self) -> Result<EscInfo> {
        self.op::<ConnectEndpoint, _>(&())
    }

    fn read_eeprom(&mut self) -> Result<[u8; EEPROM_SIZE]> {
        let data = self.op::<ReadEepromEndpoint, _>(&())?;
        data.as_slice()
            .try_into()
            .map_err(|_| anyhow!("relay returned {} EEPROM bytes", data.len()))
    }

    fn write_eeprom(&mut self, data: &[u8; EEPROM_SIZE]) -> Result<()> {
        let req = heapless::Vec::from_slice(data).map_err(|_| anyhow!("EEPROM too large"))?;
        self.op::<WriteEepromEndpoint, _>(&req)
    }

    fn read_file_name(&mut self) -> Result<Vec<u8>> {
        Ok(self.op::<ReadFileNameEndpoint, _>(&())?.to_vec())
    }

    fn read(&mut self, address: u16, len: u16) -> Result<Vec<u8>> {
        Ok(self
            .op::<ReadEndpoint, _>(&ReadReq { address, len })?
            .to_vec())
    }

    fn flash_begin(&mut self) -> Result<()> {
        self.op::<FlashBeginEndpoint, _>(&())
    }

    fn flash_chunk(&mut self, offset: u32, data: &[u8]) -> Result<()> {
        let data = heapless::Vec::from_slice(data)
            .map_err(|_| anyhow!("chunk of {} bytes too large", data.len()))?;
        self.op::<FlashChunkEndpoint, _>(&ChunkReq { offset, data })
    }

    fn flash_finish(&mut self) -> Result<()> {
        self.op::<FlashFinishEndpoint, _>(&())
    }

    fn read_firmware(&mut self, offset: u32, len: u16) -> Result<Vec<u8>> {
        Ok(self
            .op::<ReadFirmwareEndpoint, _>(&ReadFirmwareReq { offset, len })?
            .to_vec())
    }

    fn run(&mut self) -> Result<()> {
        self.op::<RunEndpoint, _>(&())
    }
}

/// A connected ESC behind the relay.
pub type Relay = Session<RelayClient>;

impl Session<RelayClient> {
    /// Open the relay on `port` and connect to the ESC bootloader. Refuses
    /// non-AM32 ESCs.
    pub fn connect(port: &str) -> Result<Self> {
        Self::new(RelayClient::open(port)?)
    }
}

/// List available serial ports.
pub fn list_ports() -> Result<Vec<String>> {
    let ports = serialport::available_ports().context("failed to list serial ports")?;

    Ok(ports.into_iter().map(|p| p.port_name).collect())
}

/// Serial ports that are relays (by USB VID:PID).
pub fn find_relays() -> Result<Vec<String>> {
    let ports = serialport::available_ports().context("failed to list serial ports")?;
    Ok(ports
        .into_iter()
        .filter(|p| {
            matches!(&p.port_type, serialport::SerialPortType::UsbPort(u)
                if (u.vid, u.pid) == RELAY_USB_VID_PID)
        })
        .map(|p| p.port_name)
        .collect())
}
