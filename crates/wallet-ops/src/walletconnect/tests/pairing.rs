use std::time::{SystemTime, UNIX_EPOCH};

use alloy::hex;
use serde_json::json;

use crate::walletconnect::crypto::encode_walletconnect_message_with_nonce;
use crate::walletconnect::uri::WalletConnectPairingUri;
use crate::walletconnect::{
    WalletConnectRelayStep, decode_walletconnect_session_proposal, start_walletconnect_pairing,
};

use super::helpers::{NOW, TOPIC, valid_uri};

#[test]
fn pairing_start_fetches_before_subscribe() {
    let start = start_walletconnect_pairing(&valid_uri(""), NOW).unwrap();

    assert_eq!(start.uri.topic, TOPIC);
    assert!(matches!(
        &start.relay_steps[0],
        WalletConnectRelayStep::FetchMessages { topic } if topic == TOPIC
    ));
    assert!(matches!(
        &start.relay_steps[1],
        WalletConnectRelayStep::Subscribe { topic } if topic == TOPIC
    ));
}

#[test]
fn decodes_encrypted_session_proposal_and_summary() {
    let pairing = WalletConnectPairingUri::parse_with_now(&valid_uri(""), NOW).unwrap();
    let proposal_request = json!({
        "id": 42,
        "jsonrpc": "2.0",
        "method": "wc_sessionPropose",
        "params": {
            "requiredNamespaces": {
                "eip155": {
                    "chains": ["eip155:1"],
                    "methods": ["eth_accounts"],
                    "events": ["accountsChanged"]
                }
            },
            "optionalNamespaces": {},
            "proposer": {
                "publicKey": hex::encode([4u8; 32]),
                "metadata": {
                    "name": "Example Dapp",
                    "description": "Example",
                    "url": "https://example.invalid",
                    "icons": []
                }
            },
            "relays": [{ "protocol": "irn" }],
            "expiryTimestamp": NOW + 60
        }
    });
    let envelope = encode_walletconnect_message_with_nonce(
        &pairing.sym_key,
        serde_json::to_string(&proposal_request).unwrap().as_bytes(),
        [7u8; 12],
    )
    .unwrap();

    let proposal = decode_walletconnect_session_proposal(&pairing, &envelope.to_base64()).unwrap();
    let summary = proposal.summary(NOW);

    assert_eq!(proposal.id, 42);
    assert_eq!(proposal.required_namespaces.len(), 1);
    assert_eq!(summary.dapp_name, "Example Dapp");
    assert!(!summary.expired);
}

#[test]
fn decodes_session_proposal_without_expiry_timestamp() {
    let pairing_expiry = NOW + 600;
    let pairing = WalletConnectPairingUri::parse_with_now(
        &valid_uri(&format!("&expiryTimestamp={pairing_expiry}")),
        NOW,
    )
    .unwrap();
    let proposal_request = json!({
        "id": 43,
        "jsonrpc": "2.0",
        "method": "wc_sessionPropose",
        "params": {
            "requiredNamespaces": {
                "eip155": {
                    "chains": ["eip155:1"],
                    "methods": ["eth_accounts"],
                    "events": ["accountsChanged"]
                }
            },
            "proposer": {
                "publicKey": hex::encode([4u8; 32]),
                "metadata": {
                    "name": "Example Dapp",
                    "description": "Example",
                    "url": "https://example.invalid",
                    "icons": []
                }
            },
            "relays": [{ "protocol": "irn" }]
        }
    });
    let envelope = encode_walletconnect_message_with_nonce(
        &pairing.sym_key,
        serde_json::to_string(&proposal_request).unwrap().as_bytes(),
        [8u8; 12],
    )
    .unwrap();

    let before_decode = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let proposal = decode_walletconnect_session_proposal(&pairing, &envelope.to_base64()).unwrap();
    let after_decode = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    assert_eq!(proposal.id, 43);
    assert_ne!(proposal.expiry_timestamp, pairing_expiry);
    assert!(proposal.expiry_timestamp >= before_decode + 300);
    assert!(proposal.expiry_timestamp <= after_decode + 300);
}
