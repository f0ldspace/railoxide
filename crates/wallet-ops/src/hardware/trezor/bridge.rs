use std::fmt;
use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use alloy::hex;
use protobuf::Enum as _;
use serde::Deserialize;
use trezor_client::transport::{ProtoMessage, Transport, error::Error as TrezorTransportError};

use super::super::HardwareDerivationError;

const TREZOR_BRIDGE_ADDR: &str = "127.0.0.1:21325";
const TREZOR_BRIDGE_HOST: &str = "127.0.0.1";
const TREZOR_BRIDGE_ORIGIN: &str = "http://localhost:8000";
const TREZOR_BRIDGE_CONNECT_TIMEOUT: Duration = Duration::from_millis(750);
const TREZOR_BRIDGE_READ_TIMEOUT: Duration = Duration::from_mins(5);
const TREZOR_BRIDGE_WRITE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Deserialize)]
pub(in crate::hardware) struct BridgeDevice {
    pub(in crate::hardware) path: String,
    pub(in crate::hardware) session: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BridgeAcquireResponse {
    session: String,
}

#[derive(Debug)]
pub(in crate::hardware) enum BridgeConnectError {
    Unavailable(String),
    NoDevice,
    DeviceBusy,
    DeviceNotUnique(usize),
    Transport(String),
}

impl BridgeConnectError {
    pub(super) const fn should_fallback(&self) -> bool {
        matches!(self, Self::Unavailable(_) | Self::NoDevice)
    }

    pub(super) fn into_hardware_error(self) -> HardwareDerivationError {
        HardwareDerivationError::TrezorBridge(self.to_string())
    }
}

impl fmt::Display for BridgeConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(error) => write!(f, "Trezor Bridge is unavailable: {error}"),
            Self::NoDevice => f.write_str("Trezor Bridge did not report a connected device"),
            Self::DeviceBusy => f.write_str(trezor_bridge_busy_message()),
            Self::DeviceNotUnique(count) => {
                write!(
                    f,
                    "Trezor Bridge reported {count} devices; connect exactly one Trezor"
                )
            }
            Self::Transport(error) => write!(f, "{error}"),
        }
    }
}

#[must_use]
pub const fn trezor_bridge_busy_message() -> &'static str {
    "Trezor Bridge reports that the device is already in use. Close Trezor Suite, browser wallet tabs, or other Trezor applications, then reconnect the device and retry."
}

struct BridgeHttpResponse {
    status: u16,
    body: Vec<u8>,
}

pub(super) struct BridgeTransport {
    session: String,
    pending_message: Option<ProtoMessage>,
    released: bool,
}

impl BridgeTransport {
    pub(super) fn connect_unique() -> Result<Self, BridgeConnectError> {
        let response = bridge_http_post(
            &bridge_path(&["enumerate"]),
            None,
            BridgeHttpErrorMode::Unavailable,
        )?;
        ensure_success(response.status, &response.body).map_err(BridgeConnectError::Transport)?;
        let devices: Vec<BridgeDevice> = serde_json::from_slice(&response.body)
            .map_err(|error| BridgeConnectError::Transport(error.to_string()))?;
        let device = select_bridge_device(&devices)?;
        let response = bridge_http_post(
            &bridge_path(&["acquire", &device.path, "null"]),
            None,
            BridgeHttpErrorMode::Transport,
        )?;
        ensure_success(response.status, &response.body).map_err(BridgeConnectError::Transport)?;
        let response: BridgeAcquireResponse = serde_json::from_slice(&response.body)
            .map_err(|error| BridgeConnectError::Transport(error.to_string()))?;
        Ok(Self {
            session: response.session,
            pending_message: None,
            released: false,
        })
    }

    fn release(&mut self) -> Result<(), TrezorTransportError> {
        if self.released {
            return Ok(());
        }
        let response = bridge_http_post(
            &bridge_path(&["release", &self.session]),
            None,
            BridgeHttpErrorMode::Transport,
        )
        .map_err(|error| transport_io_error(error.to_string()))?;
        ensure_success(response.status, &response.body).map_err(transport_io_error)?;
        self.released = true;
        Ok(())
    }

    fn call(&self, message: ProtoMessage) -> Result<ProtoMessage, TrezorTransportError> {
        let body = encode_bridge_message(message);
        let response = bridge_http_post(
            &bridge_path(&["call", &self.session]),
            Some(&body),
            BridgeHttpErrorMode::Transport,
        )
        .map_err(|error| transport_io_error(error.to_string()))?;
        ensure_success(response.status, &response.body).map_err(transport_io_error)?;
        let body = std::str::from_utf8(&response.body)
            .map_err(|error| transport_io_error(error.to_string()))?
            .trim();
        let data = hex::decode(body).map_err(|error| transport_io_error(error.to_string()))?;
        decode_bridge_message(&data)
    }
}

impl Drop for BridgeTransport {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

impl Transport for BridgeTransport {
    fn session_begin(&mut self) -> Result<(), TrezorTransportError> {
        Ok(())
    }

    fn session_end(&mut self) -> Result<(), TrezorTransportError> {
        self.release()
    }

    fn write_message(&mut self, message: ProtoMessage) -> Result<(), TrezorTransportError> {
        self.pending_message = Some(message);
        Ok(())
    }

    fn read_message(&mut self) -> Result<ProtoMessage, TrezorTransportError> {
        let message = self
            .pending_message
            .take()
            .ok_or_else(|| transport_io_error("Trezor Bridge read requested before write"))?;
        self.call(message)
    }
}

#[derive(Clone, Copy)]
enum BridgeHttpErrorMode {
    Unavailable,
    Transport,
}

fn bridge_http_post(
    path: &str,
    body: Option<&str>,
    error_mode: BridgeHttpErrorMode,
) -> Result<BridgeHttpResponse, BridgeConnectError> {
    let addr: SocketAddr = TREZOR_BRIDGE_ADDR
        .parse()
        .expect("Trezor Bridge socket address is valid");
    let mut stream = TcpStream::connect_timeout(&addr, TREZOR_BRIDGE_CONNECT_TIMEOUT)
        .map_err(|error| bridge_io_error(error_mode, &error))?;
    stream
        .set_read_timeout(Some(TREZOR_BRIDGE_READ_TIMEOUT))
        .map_err(|error| bridge_io_error(error_mode, &error))?;
    stream
        .set_write_timeout(Some(TREZOR_BRIDGE_WRITE_TIMEOUT))
        .map_err(|error| bridge_io_error(error_mode, &error))?;

    let body = body.unwrap_or("");
    let request = format!(
        "POST {path} HTTP/1.0\r\nHost: {TREZOR_BRIDGE_HOST}\r\nOrigin: {TREZOR_BRIDGE_ORIGIN}\r\nUser-Agent: railgun-wallet\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|error| bridge_io_error(error_mode, &error))?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|error| bridge_io_error(error_mode, &error))?;
    parse_http_response(&response).map_err(BridgeConnectError::Transport)
}

fn bridge_io_error(error_mode: BridgeHttpErrorMode, error: &std::io::Error) -> BridgeConnectError {
    match error_mode {
        BridgeHttpErrorMode::Unavailable => BridgeConnectError::Unavailable(error.to_string()),
        BridgeHttpErrorMode::Transport => BridgeConnectError::Transport(error.to_string()),
    }
}

fn parse_http_response(response: &[u8]) -> Result<BridgeHttpResponse, String> {
    let Some(header_end) = response.windows(4).position(|window| window == b"\r\n\r\n") else {
        return Err("Trezor Bridge returned an invalid HTTP response".to_owned());
    };
    let headers = std::str::from_utf8(&response[..header_end])
        .map_err(|error| format!("Trezor Bridge returned non-UTF-8 headers: {error}"))?;
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|status| status.parse::<u16>().ok())
        .ok_or_else(|| "Trezor Bridge returned an invalid HTTP status".to_owned())?;
    Ok(BridgeHttpResponse {
        status,
        body: response[header_end + 4..].to_vec(),
    })
}

fn ensure_success(status: u16, body: &[u8]) -> Result<(), String> {
    if (200..300).contains(&status) {
        return Ok(());
    }
    let body = String::from_utf8_lossy(body);
    Err(format!("Trezor Bridge HTTP {status}: {body}"))
}

pub(in crate::hardware) fn select_bridge_device(
    devices: &[BridgeDevice],
) -> Result<BridgeDevice, BridgeConnectError> {
    match devices {
        [] => Err(BridgeConnectError::NoDevice),
        [device] if device.session.is_some() => Err(BridgeConnectError::DeviceBusy),
        [device] => Ok(device.clone()),
        _ => Err(BridgeConnectError::DeviceNotUnique(devices.len())),
    }
}

fn bridge_path(segments: &[&str]) -> String {
    let mut path = String::new();
    for segment in segments {
        path.push('/');
        percent_encode_path_segment(segment, &mut path);
    }
    path
}

fn percent_encode_path_segment(segment: &str, output: &mut String) {
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            output.push(char::from(byte));
        } else {
            output.push('%');
            output.push(char::from(hex_digit(byte >> 4)));
            output.push(char::from(hex_digit(byte & 0x0f)));
        }
    }
}

const fn hex_digit(value: u8) -> u8 {
    match value {
        0..=9 => b'0' + value,
        10..=15 => b'A' + (value - 10),
        _ => b'0',
    }
}

pub(in crate::hardware) fn encode_bridge_message(message: ProtoMessage) -> String {
    let message_type = message.message_type() as u16;
    let payload = message.into_payload();
    let mut data = Vec::with_capacity(6 + payload.len());
    data.extend_from_slice(&message_type.to_be_bytes());
    data.extend_from_slice(
        &u32::try_from(payload.len())
            .expect("Trezor protobuf payload length fits in u32")
            .to_be_bytes(),
    );
    data.extend_from_slice(&payload);
    hex::encode(data)
}

pub(in crate::hardware) fn decode_bridge_message(
    data: &[u8],
) -> Result<ProtoMessage, TrezorTransportError> {
    if data.len() < 6 {
        return Err(TrezorTransportError::UnexpectedChunkSizeFromDevice(
            data.len(),
        ));
    }
    let message_type_id = u16::from_be_bytes([data[0], data[1]]);
    let data_len = u32::from_be_bytes([data[2], data[3], data[4], data[5]]) as usize;
    if data.len() != 6 + data_len {
        return Err(TrezorTransportError::UnexpectedChunkSizeFromDevice(
            data.len(),
        ));
    }
    let message_type = trezor_client::protos::MessageType::from_i32(i32::from(message_type_id))
        .ok_or_else(|| TrezorTransportError::InvalidMessageType(u32::from(message_type_id)))?;
    Ok(ProtoMessage::new(message_type, data[6..].to_vec()))
}

fn transport_io_error(message: impl Into<String>) -> TrezorTransportError {
    TrezorTransportError::IO(std::io::Error::other(message.into()))
}
