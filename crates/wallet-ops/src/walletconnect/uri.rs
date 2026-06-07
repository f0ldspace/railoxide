use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

use alloy::hex;
use url::form_urlencoded;

use super::{Result, WalletConnectError};

pub const WALLETCONNECT_REQUIRED_PAIRING_METHOD: &str = "wc_sessionPropose";
pub const WALLETCONNECT_IRN_RELAY_PROTOCOL: &str = "irn";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletConnectPairingUri {
    pub topic: String,
    pub version: u8,
    pub sym_key: [u8; 32],
    pub relay_protocol: String,
    pub methods: BTreeSet<String>,
    pub expiry_timestamp: Option<u64>,
}

impl WalletConnectPairingUri {
    pub fn parse(input: &str) -> Result<Self> {
        Self::parse_with_now(input, current_unix_seconds())
    }

    pub fn parse_with_now(input: &str, now_unix_seconds: u64) -> Result<Self> {
        let input = input.trim();
        let Some(rest) = input.strip_prefix("wc:") else {
            return Err(WalletConnectError::InvalidUri(
                "URI must start with wc:".to_owned(),
            ));
        };

        let (authority, query) = rest.split_once('?').ok_or_else(|| {
            WalletConnectError::InvalidUri("URI must include query parameters".to_owned())
        })?;
        let (topic, version) = authority.split_once('@').ok_or_else(|| {
            WalletConnectError::InvalidUri("URI must include topic and version".to_owned())
        })?;

        if topic.trim().is_empty() {
            return Err(WalletConnectError::InvalidUri(
                "pairing topic is required".to_owned(),
            ));
        }
        validate_pairing_topic(topic)?;

        let version = version.parse::<u8>().map_err(|_| {
            WalletConnectError::InvalidUri("WalletConnect version must be numeric".to_owned())
        })?;
        if version != 2 {
            return Err(WalletConnectError::InvalidUri(
                "only WalletConnect v2 URIs are supported".to_owned(),
            ));
        }

        let mut sym_key = None;
        let mut relay_protocol = None;
        let mut methods = None;
        let mut expiry_timestamp = None;

        for (key, value) in form_urlencoded::parse(query.as_bytes()) {
            match key.as_ref() {
                "symKey" => sym_key = Some(parse_sym_key(&value)?),
                "relay-protocol" => relay_protocol = Some(value.into_owned()),
                "methods" => methods = Some(parse_methods(&value)),
                "expiryTimestamp" => {
                    let parsed = value.parse::<u64>().map_err(|_| {
                        WalletConnectError::InvalidUri("expiryTimestamp must be numeric".to_owned())
                    })?;
                    expiry_timestamp = Some(parsed);
                }
                _ => {}
            }
        }

        let sym_key = sym_key
            .ok_or_else(|| WalletConnectError::InvalidUri("symKey is required".to_owned()))?;
        let relay_protocol = relay_protocol.ok_or_else(|| {
            WalletConnectError::InvalidUri("relay-protocol is required".to_owned())
        })?;
        if relay_protocol != WALLETCONNECT_IRN_RELAY_PROTOCOL {
            return Err(WalletConnectError::InvalidUri(
                "only IRN relay protocol is supported".to_owned(),
            ));
        }

        let methods = match methods {
            Some(methods) => {
                if !methods.contains(WALLETCONNECT_REQUIRED_PAIRING_METHOD) {
                    return Err(WalletConnectError::InvalidUri(
                        "methods must include wc_sessionPropose".to_owned(),
                    ));
                }
                methods
            }
            None => BTreeSet::new(),
        };

        if expiry_timestamp.is_some_and(|expiry| expiry <= now_unix_seconds) {
            return Err(WalletConnectError::ExpiredUri);
        }

        Ok(Self {
            topic: topic.to_owned(),
            version,
            sym_key,
            relay_protocol,
            methods,
            expiry_timestamp,
        })
    }
}

fn validate_pairing_topic(value: &str) -> Result<()> {
    let bytes = hex::decode(value).map_err(|_| {
        WalletConnectError::InvalidUri("pairing topic must be a 32-byte hex value".to_owned())
    })?;
    if bytes.len() == 32 {
        Ok(())
    } else {
        Err(WalletConnectError::InvalidUri(
            "pairing topic must be a 32-byte hex value".to_owned(),
        ))
    }
}

fn parse_sym_key(value: &str) -> Result<[u8; 32]> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    let bytes = hex::decode(value).map_err(|_| {
        WalletConnectError::InvalidUri("symKey must be a 32-byte hex value".to_owned())
    })?;
    bytes.try_into().map_err(|_| {
        WalletConnectError::InvalidUri("symKey must be a 32-byte hex value".to_owned())
    })
}

fn parse_methods(value: &str) -> BTreeSet<String> {
    value
        .split(',')
        .map(str::trim)
        .map(|method| method.trim_matches(['[', ']']))
        .filter(|method| !method.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}
