use alloy::hex;

use crate::walletconnect::WalletConnectError;
use crate::walletconnect::uri::WalletConnectPairingUri;

use super::helpers::{NOW, SYM_KEY, TOPIC, valid_uri, valid_uri_without_methods};

#[test]
fn parses_valid_uri_without_expiry() {
    let uri = WalletConnectPairingUri::parse_with_now(&valid_uri(""), NOW).unwrap();

    assert_eq!(uri.topic, TOPIC);
    assert_eq!(uri.version, 2);
    assert_eq!(hex::encode(uri.sym_key), SYM_KEY);
    assert_eq!(uri.relay_protocol, "irn");
    assert!(uri.methods.contains("wc_sessionPropose"));
    assert_eq!(uri.expiry_timestamp, None);
}

#[test]
fn parses_valid_uri_with_unexpired_expiry() {
    let uri =
        WalletConnectPairingUri::parse_with_now(&valid_uri("&expiryTimestamp=1700000060"), NOW)
            .unwrap();

    assert_eq!(uri.expiry_timestamp, Some(1_700_000_060));
}

#[test]
fn accepts_uri_without_methods() {
    let uri = WalletConnectPairingUri::parse_with_now(&valid_uri_without_methods(""), NOW).unwrap();

    assert!(uri.methods.is_empty());
}

#[test]
fn accepts_aave_uri_without_methods() {
    let uri = WalletConnectPairingUri::parse_with_now(
        "wc:2526e5fdd74bf250d7b7a3b2539677b3e76a2494a03a9dd344877523cef29dee@2?relay-protocol=irn&symKey=153c82e84a65346da926a473aca4015ba01d997111998942fa11b64a5d005bb9&expiryTimestamp=1780676005",
        NOW,
    )
    .unwrap();

    assert_eq!(
        uri.topic,
        "2526e5fdd74bf250d7b7a3b2539677b3e76a2494a03a9dd344877523cef29dee"
    );
    assert!(uri.methods.is_empty());
    assert_eq!(uri.expiry_timestamp, Some(1_780_676_005));
}

#[test]
fn parses_bracketed_methods_format() {
    let uri = format!(
        "wc:{TOPIC}@2?relay-protocol=irn&symKey={SYM_KEY}&methods=[wc_sessionPropose],[wc_authRequest,wc_authBatchRequest]"
    );
    let uri = WalletConnectPairingUri::parse_with_now(&uri, NOW).unwrap();

    assert!(uri.methods.contains("wc_sessionPropose"));
    assert!(uri.methods.contains("wc_authRequest"));
    assert!(uri.methods.contains("wc_authBatchRequest"));
}

#[test]
fn rejects_empty_methods_when_parameter_is_present() {
    let uri = format!("wc:{TOPIC}@2?relay-protocol=irn&symKey={SYM_KEY}&methods=");

    assert!(matches!(
        WalletConnectPairingUri::parse_with_now(&uri, NOW),
        Err(WalletConnectError::InvalidUri(message)) if message.contains("wc_sessionPropose")
    ));
}

#[test]
fn rejects_uri_without_session_proposal_method() {
    let uri = format!("wc:{TOPIC}@2?relay-protocol=irn&symKey={SYM_KEY}&methods=wc_sessionPing");

    assert!(matches!(
        WalletConnectPairingUri::parse_with_now(&uri, NOW),
        Err(WalletConnectError::InvalidUri(message)) if message.contains("wc_sessionPropose")
    ));
}

#[test]
fn rejects_non_irn_relay_protocol() {
    let uri =
        format!("wc:{TOPIC}@2?relay-protocol=custom&symKey={SYM_KEY}&methods=wc_sessionPropose");

    assert!(matches!(
        WalletConnectPairingUri::parse_with_now(&uri, NOW),
        Err(WalletConnectError::InvalidUri(message)) if message.contains("IRN")
    ));
}

#[test]
fn rejects_expired_uri() {
    assert!(matches!(
        WalletConnectPairingUri::parse_with_now(&valid_uri("&expiryTimestamp=1699999999"), NOW),
        Err(WalletConnectError::ExpiredUri)
    ));
}

#[test]
fn rejects_malformed_pairing_topic() {
    let malformed =
        format!("wc:not-a-topic@2?relay-protocol=irn&symKey={SYM_KEY}&methods=wc_sessionPropose");
    let short = format!("wc:0123@2?relay-protocol=irn&symKey={SYM_KEY}");

    assert!(matches!(
        WalletConnectPairingUri::parse_with_now(&malformed, NOW),
        Err(WalletConnectError::InvalidUri(message)) if message.contains("pairing topic")
    ));
    assert!(matches!(
        WalletConnectPairingUri::parse_with_now(&short, NOW),
        Err(WalletConnectError::InvalidUri(message)) if message.contains("32-byte hex")
    ));
}
