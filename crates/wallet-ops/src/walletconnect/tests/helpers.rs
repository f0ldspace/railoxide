use std::collections::{BTreeMap, BTreeSet};

use alloy::hex;
use alloy::primitives::address;
use serde_json::{Value, json};

use crate::vault::{
    PublicAccountMetadata, PublicAccountScope, PublicAccountSource, PublicAccountStatus,
    WalletConnectPeerMetadata, WalletConnectRelayIdentity,
};
use crate::walletconnect::crypto::decode_walletconnect_message;
use crate::walletconnect::{
    WalletConnectEnvelope, WalletConnectNamespaceProposal, WalletConnectSessionProposal,
    approve_walletconnect_session,
};

pub(super) const NOW: u64 = 1_700_000_000;
pub(super) const TOPIC: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
pub(super) const SYM_KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

pub(super) fn decode_encrypted_json(sym_key: &[u8; 32], message: &str) -> Value {
    let envelope = WalletConnectEnvelope::from_base64(message).unwrap();
    let plaintext = decode_walletconnect_message(sym_key, &envelope).unwrap();
    serde_json::from_slice(&plaintext).unwrap()
}

pub(super) fn valid_uri(extra: &str) -> String {
    format!("wc:{TOPIC}@2?relay-protocol=irn&symKey={SYM_KEY}&methods=wc_sessionPropose{extra}")
}

pub(super) fn valid_uri_without_methods(extra: &str) -> String {
    format!("wc:{TOPIC}@2?relay-protocol=irn&symKey={SYM_KEY}{extra}")
}

pub(super) fn typed_data_payload(chain_id: &Value) -> Value {
    json!({
        "types": {
            "EIP712Domain": [
                { "name": "name", "type": "string" },
                { "name": "version", "type": "string" },
                { "name": "chainId", "type": "uint256" }
            ],
            "Message": [
                { "name": "contents", "type": "string" }
            ]
        },
        "primaryType": "Message",
        "domain": {
            "name": "RailOxide",
            "version": "1",
            "chainId": chain_id
        },
        "message": {
            "contents": "hello"
        }
    })
}

pub(super) fn supported_chains(chains: &[u64]) -> BTreeSet<u64> {
    chains.iter().copied().collect()
}

pub(super) fn namespace(
    chains: &[&str],
    methods: &[&str],
    events: &[&str],
) -> WalletConnectNamespaceProposal {
    WalletConnectNamespaceProposal {
        chains: chains.iter().map(ToString::to_string).collect(),
        methods: methods.iter().map(ToString::to_string).collect(),
        events: events.iter().map(ToString::to_string).collect(),
    }
}

pub(super) fn test_public_account(scope: PublicAccountScope) -> PublicAccountMetadata {
    PublicAccountMetadata {
        public_account_uuid: "public-account".to_owned(),
        address: address!("1111111111111111111111111111111111111111"),
        label: Some("WalletConnect account".to_owned()),
        source: PublicAccountSource::Imported,
        scope,
        derivation_index: None,
        hardware_descriptor: None,
        status: PublicAccountStatus::Active,
        display_order: 0,
    }
}

pub(super) fn test_proposal(
    required_namespaces: BTreeMap<String, WalletConnectNamespaceProposal>,
) -> WalletConnectSessionProposal {
    WalletConnectSessionProposal {
        id: 42,
        pairing_topic: "pairing-topic".to_owned(),
        proposer_public_key: hex::encode(
            x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from([9u8; 32])).to_bytes(),
        ),
        relay_protocol: "irn".to_owned(),
        peer_metadata: WalletConnectPeerMetadata {
            name: "Example Dapp".to_owned(),
            description: "Example".to_owned(),
            url: "https://example.invalid".to_owned(),
            icons: vec![],
        },
        required_namespaces,
        optional_namespaces: BTreeMap::new(),
        expiry_timestamp: NOW + 300,
    }
}

pub(super) fn approved_request_session(
    methods: &[&str],
) -> (
    crate::vault::WalletConnectSessionRecord,
    PublicAccountMetadata,
) {
    let mut required = BTreeMap::new();
    required.insert(
        "eip155".to_owned(),
        namespace(&["eip155:1"], methods, &["accountsChanged", "chainChanged"]),
    );
    let proposal = test_proposal(required);
    let relay_identity = WalletConnectRelayIdentity {
        signing_key: [8u8; 32],
        client_id: "relay-client".to_owned(),
    };
    let account = test_public_account(PublicAccountScope::Global);
    let approval = approve_walletconnect_session(
        &proposal,
        &[1u8; 32],
        &relay_identity,
        &account,
        &supported_chains(&[1]),
        "request-session",
        NOW,
    )
    .unwrap();
    (approval.session, account)
}
