use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use alloy::hex;
use alloy::primitives::{U256, address};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use zeroize::Zeroize;

use crate::vault::{
    PublicAccountMetadata, PublicAccountScope, PublicAccountSource, PublicAccountStatus,
    WalletConnectPeerMetadata, WalletConnectRelayIdentity, WalletConnectSessionAccountResolution,
};

use super::crypto::{
    decode_walletconnect_message, derive_walletconnect_session_sym_key,
    derive_walletconnect_session_topic, encode_walletconnect_message_with_nonce,
    hash_walletconnect_key,
};
use super::relay::{
    WALLETCONNECT_DEFAULT_PROJECT_ID, WALLETCONNECT_RELAY_URL, WalletConnectJsonRpcId,
    WalletConnectJsonRpcResponse, WalletConnectRelayClient, WalletConnectRelayClientAuth,
    WalletConnectRelayConfig, WalletConnectRelayRpc, WalletConnectRelaySubscriptionRequest,
    relay_response_id_matches, sanitize_walletconnect_relay_error_message,
};
use super::session::{WC_SESSION_SETTLE, WalletConnectApprovalMessages, WalletConnectRelayStep};
use super::uri::WalletConnectPairingUri;
use super::{
    WalletConnectErc20CallSummary, WalletConnectError, WalletConnectLifecycleRequestOutcome,
    WalletConnectNamespaceProposal, WalletConnectParsedRequest, WalletConnectPendingRequestQueue,
    WalletConnectProposalRejectionReason, WalletConnectRelayLifecycle,
    WalletConnectRequestErrorKind, WalletConnectSessionProposal, WalletConnectTerminalLifecycleEnd,
    approve_walletconnect_session, build_walletconnect_disconnect_plan,
    build_walletconnect_jsonrpc_error, build_walletconnect_session_event,
    decode_walletconnect_session_proposal, handle_walletconnect_lifecycle_request,
    negotiate_walletconnect_namespaces, parse_walletconnect_session_request,
    reject_walletconnect_session_proposal, start_walletconnect_pairing,
    validate_walletconnect_session_request,
};

const NOW: u64 = 1_700_000_000;
const TOPIC: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const SYM_KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn decode_encrypted_json(sym_key: &[u8; 32], message: &str) -> Value {
    let envelope = super::crypto::WalletConnectEnvelope::from_base64(message).unwrap();
    let plaintext = decode_walletconnect_message(sym_key, &envelope).unwrap();
    serde_json::from_slice(&plaintext).unwrap()
}

fn valid_uri(extra: &str) -> String {
    format!("wc:{TOPIC}@2?relay-protocol=irn&symKey={SYM_KEY}&methods=wc_sessionPropose{extra}")
}

fn valid_uri_without_methods(extra: &str) -> String {
    format!("wc:{TOPIC}@2?relay-protocol=irn&symKey={SYM_KEY}{extra}")
}

fn typed_data_payload(chain_id: Value) -> Value {
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

#[test]
fn serializes_relay_publish_rpc() {
    let rpc = WalletConnectRelayRpc::Publish {
        topic: "topic-a".to_owned(),
        message: "payload".to_owned(),
        ttl: 300,
        tag: 1102,
    };
    let serialized = serde_json::to_string(&rpc.request(7)).unwrap();

    assert_eq!(
        serialized,
        r#"{"id":"7","jsonrpc":"2.0","method":"irn_publish","params":{"message":"payload","tag":1102,"topic":"topic-a","ttl":300}}"#
    );
}

#[test]
fn relay_request_id_matches_string_and_number_responses() {
    let id = 1_700_000_000_000_000_123_u64;
    let rpc = WalletConnectRelayRpc::Subscribe {
        topic: "topic-a".to_owned(),
    };
    let serialized = serde_json::to_value(rpc.request(id)).unwrap();

    assert_eq!(serialized["id"], json!(id.to_string()));
    assert!(relay_response_id_matches(
        &json!({ "id": id.to_string(), "jsonrpc": "2.0", "result": true }),
        id,
    ));
    assert!(relay_response_id_matches(
        &json!({ "id": id, "jsonrpc": "2.0", "result": true }),
        id,
    ));
}

#[test]
fn relay_subscription_error_is_not_treated_as_success() {
    let response = WalletConnectJsonRpcResponse::<Value> {
        id: 9,
        jsonrpc: "2.0".to_owned(),
        result: None,
        error: Some(super::relay::WalletConnectJsonRpcError {
            code: -32_000,
            message: "subscription failed".to_owned(),
        }),
    };

    assert!(matches!(
        response.into_result(),
        Err(WalletConnectError::Relay(message)) if message.contains("subscription failed")
    ));
}

#[test]
fn default_relay_config_uses_bundled_project_id() {
    assert_eq!(
        WalletConnectRelayConfig::default().project_id,
        WALLETCONNECT_DEFAULT_PROJECT_ID
    );
}

#[test]
fn relay_client_auth_reuses_client_id_for_same_identity() {
    let identity = [7u8; 32];
    let first = WalletConnectRelayClientAuth::from_signing_key(identity);
    let second = WalletConnectRelayClientAuth::from_signing_key(identity);

    assert_eq!(first.client_id, second.client_id);
    assert!(first.client_id.starts_with("did:key:z"));
    assert!(
        !first
            .bearer_token(WALLETCONNECT_RELAY_URL, 60)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn relay_client_auth_secret_debug_output_is_redacted_and_zeroizable() {
    let mut auth = WalletConnectRelayClientAuth::from_signing_key([7u8; 32]);

    let debug = format!("{auth:?}");

    assert!(debug.contains("<redacted>"));
    assert!(debug.contains(&auth.client_id));
    assert!(!debug.contains("[7, 7"));

    auth.zeroize();

    assert_eq!(auth.signing_key, [0u8; 32]);
    assert!(auth.client_id.is_empty());
}

#[test]
fn relay_client_auth_jwt_uses_client_auth_claims() {
    let auth = WalletConnectRelayClientAuth::from_signing_key([7u8; 32]);
    let token = auth.client_auth_jwt(WALLETCONNECT_RELAY_URL, 60).unwrap();
    let parts = token.split('.').collect::<Vec<_>>();
    assert_eq!(parts.len(), 3);
    let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();

    assert_eq!(claims["iss"], auth.client_id);
    assert_ne!(claims["sub"], auth.client_id);
    assert_eq!(claims["aud"], WALLETCONNECT_RELAY_URL);
    assert_eq!(claims["act"], "client_auth");
    assert!(claims["iat"].as_u64().is_some());
    assert!(claims["exp"].as_u64().unwrap() > claims["iat"].as_u64().unwrap());
}

#[test]
fn relay_client_connection_query_includes_project_id_and_auth() {
    let auth = WalletConnectRelayClientAuth::from_signing_key([8u8; 32]);
    let client = WalletConnectRelayClient::new(
        WalletConnectRelayConfig {
            project_id: "project-override".to_owned(),
        },
        auth,
    );
    let query = client.connection_query().unwrap();

    assert!(
        query
            .iter()
            .any(|(key, value)| key == "projectId" && value == "project-override")
    );
    assert!(
        query
            .iter()
            .any(|(key, value)| key == "auth" && value.split('.').count() == 3)
    );
    assert!(
        query
            .iter()
            .any(|(key, value)| key == "ua" && value.starts_with("wc-2/rust-"))
    );
    assert!(
        query
            .iter()
            .any(|(key, value)| key == "useOnCloseEvent" && value == "true")
    );
}

#[test]
fn relay_error_sanitizer_removes_auth_jwts_and_full_queries() {
    let jwt = "eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCJ9.eyJhdWQiOiJ3c3M6Ly9yZWxheS53YWxsZXRjb25uZWN0Lm9yZyJ9.c2lnbmF0dXJlLXBheWxvYWQ";
    let raw = format!(
        "websocket upgrade failed: Request {{ url: wss://user:pass@relay.walletconnect.org/?auth={jwt}&projectId=project-override&ua=wc-2/rust-test&useOnCloseEvent=true }} auth={jwt} symKey=000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"
    );

    let sanitized = sanitize_walletconnect_relay_error_message(&raw);

    assert!(sanitized.contains("wss://relay.walletconnect.org/"));
    assert!(!sanitized.contains("user:pass"));
    assert!(!sanitized.contains("auth="));
    assert!(!sanitized.contains("projectId="));
    assert!(!sanitized.contains("useOnCloseEvent="));
    assert!(!sanitized.contains("symKey="));
    assert!(!sanitized.contains(jwt));
    assert!(
        !sanitized.contains("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f")
    );
}

#[test]
fn relay_subscription_request_is_acknowledged() {
    let request = json!({
        "id": 44,
        "jsonrpc": "2.0",
        "method": "irn_subscription",
        "params": {
            "id": "sub-1",
            "topic": "topic-a",
            "message": "payload"
        }
    });
    let parsed = WalletConnectRelaySubscriptionRequest::parse(&request)
        .unwrap()
        .expect("subscription request");
    let ack = parsed.ack();

    assert_eq!(parsed.params.topic, "topic-a");
    assert_eq!(parsed.params.message, "payload");
    assert_eq!(serde_json::to_value(&ack).unwrap()["id"], json!(44));
    assert_eq!(ack.result, Some(json!(true)));
}

#[test]
fn relay_subscription_request_accepts_string_jsonrpc_id_and_aave_metadata() {
    let request = json!({
        "id": "aave-push-id",
        "jsonrpc": "2.0",
        "method": "irn_subscription",
        "params": {
            "id": "sub-aave",
            "data": {
                "topic": "5d144cf02e8bdcfc0c2cc59600f99c336a41aa28a385c54deb4d2d6a34aea64b",
                "message": "encrypted-approve-payload",
                "tag": 1108,
                "publishedAt": 1700000000u64,
                "attestation": {
                    "origin": "https://app.aave.com"
                }
            }
        }
    });
    let parsed = WalletConnectRelaySubscriptionRequest::parse(&request)
        .unwrap()
        .expect("subscription request");
    let ack = serde_json::to_value(parsed.ack()).unwrap();

    assert_eq!(parsed.id, WalletConnectJsonRpcId::from("aave-push-id"));
    assert_eq!(parsed.params.id, "sub-aave");
    assert_eq!(
        parsed.params.topic,
        "5d144cf02e8bdcfc0c2cc59600f99c336a41aa28a385c54deb4d2d6a34aea64b"
    );
    assert_eq!(parsed.params.message, "encrypted-approve-payload");
    assert_eq!(ack["id"], json!("aave-push-id"));
    assert_eq!(ack["result"], json!(true));
}

#[test]
fn relay_subscription_request_accepts_walletconnect_data_shape() {
    let request = json!({
        "id": 45,
        "jsonrpc": "2.0",
        "method": "irn_subscription",
        "params": {
            "id": "sub-2",
            "data": {
                "topic": "topic-b",
                "message": "payload-b",
                "publishedAt": 1700000000u64
            }
        }
    });
    let parsed = WalletConnectRelaySubscriptionRequest::parse(&request)
        .unwrap()
        .expect("subscription request");

    assert_eq!(parsed.params.id, "sub-2");
    assert_eq!(parsed.params.topic, "topic-b");
    assert_eq!(parsed.params.message, "payload-b");
    assert_eq!(parsed.ack().result, Some(json!(true)));
}

#[test]
fn approval_relay_steps_fetch_subscribe_then_publish_both_messages() {
    let pairing_sym_key = [3u8; 32];
    let session_sym_key = [4u8; 32];
    let messages = WalletConnectApprovalMessages::new(
        1,
        2,
        "irn",
        "responder-public-key",
        json!({
            "relay": { "protocol": "irn" },
            "namespaces": {},
            "expiry": 1700000600,
        }),
    );
    let steps = messages
        .encrypted_relay_steps(
            "pairing-topic",
            &pairing_sym_key,
            "session-topic",
            &session_sym_key,
            300,
        )
        .unwrap();

    assert!(matches!(
        &steps[0],
        WalletConnectRelayStep::FetchMessages { topic } if topic == "session-topic"
    ));
    assert!(matches!(
        &steps[1],
        WalletConnectRelayStep::Subscribe { topic } if topic == "session-topic"
    ));

    let settle = steps[2].rpc().request(10);
    assert_eq!(settle.method, "irn_publish");
    assert_eq!(settle.params["topic"], "session-topic");
    assert!(serde_json::from_str::<Value>(settle.params["message"].as_str().unwrap()).is_err());
    let settle_message =
        decode_encrypted_json(&session_sym_key, settle.params["message"].as_str().unwrap());
    assert_eq!(settle_message["method"], WC_SESSION_SETTLE);

    let proposal = steps[3].rpc().request(11);
    assert_eq!(proposal.params["topic"], "pairing-topic");
    assert!(serde_json::from_str::<Value>(proposal.params["message"].as_str().unwrap()).is_err());
    let proposal_message = decode_encrypted_json(
        &pairing_sym_key,
        proposal.params["message"].as_str().unwrap(),
    );
    assert_eq!(
        proposal_message["result"]["responderPublicKey"],
        "responder-public-key"
    );
    assert_eq!(proposal_message["result"]["relay"]["protocol"], "irn");
}

#[test]
fn encrypted_envelope_round_trips_and_decodes_from_base64() {
    let sym_key = [1u8; 32];
    let nonce = [2u8; 12];
    let envelope =
        encode_walletconnect_message_with_nonce(&sym_key, br#"{"id":1}"#, nonce).unwrap();
    let encoded = envelope.to_base64();
    let decoded = super::crypto::WalletConnectEnvelope::from_base64(&encoded).unwrap();
    let plaintext = decode_walletconnect_message(&sym_key, &decoded).unwrap();

    assert_eq!(plaintext, br#"{"id":1}"#);
}

#[test]
fn malformed_envelope_or_wrong_key_is_rejected() {
    assert!(super::crypto::WalletConnectEnvelope::from_base64("not base64").is_err());

    let envelope =
        encode_walletconnect_message_with_nonce(&[1u8; 32], b"payload", [2u8; 12]).unwrap();
    assert!(decode_walletconnect_message(&[3u8; 32], &envelope).is_err());
}

#[test]
fn session_topic_is_hash_of_derived_symmetric_key() {
    let private_a = [1u8; 32];
    let public_a = x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(private_a));
    let private_b = [2u8; 32];
    let public_b = x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(private_b));

    let sym_key_a = derive_walletconnect_session_sym_key(&private_a, public_b.as_bytes()).unwrap();
    let sym_key_b = derive_walletconnect_session_sym_key(&private_b, public_a.as_bytes()).unwrap();

    assert_eq!(sym_key_a, sym_key_b);
    assert_eq!(
        derive_walletconnect_session_topic(&sym_key_a),
        hash_walletconnect_key(&sym_key_a)
    );
}

#[test]
fn rejects_all_zero_x25519_shared_secret() {
    assert!(matches!(
        derive_walletconnect_session_sym_key(&[1u8; 32], &[0u8; 32]),
        Err(WalletConnectError::Crypto)
    ));
}

fn supported_chains(chains: &[u64]) -> BTreeSet<u64> {
    chains.iter().copied().collect()
}

fn namespace(chains: &[&str], methods: &[&str], events: &[&str]) -> WalletConnectNamespaceProposal {
    WalletConnectNamespaceProposal {
        chains: chains.iter().map(ToString::to_string).collect(),
        methods: methods.iter().map(ToString::to_string).collect(),
        events: events.iter().map(ToString::to_string).collect(),
    }
}

fn test_public_account(scope: PublicAccountScope) -> PublicAccountMetadata {
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

fn test_proposal(
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

#[test]
fn required_caip2_namespace_key_declares_chain() {
    let mut required = BTreeMap::new();
    required.insert(
        "eip155:10".to_owned(),
        namespace(&[], &["eth_accounts"], &["chainChanged"]),
    );

    let negotiated = negotiate_walletconnect_namespaces(
        &required,
        &BTreeMap::new(),
        &supported_chains(&[10]),
        address!("1111111111111111111111111111111111111111"),
        PublicAccountSource::Imported,
    )
    .unwrap();
    let approved = negotiated
        .approved_namespaces
        .get("eip155:10")
        .expect("approved keyed namespace");

    assert_eq!(approved.chains, vec!["eip155:10"]);
    assert_eq!(
        approved.accounts,
        vec!["eip155:10:0x1111111111111111111111111111111111111111"]
    );
}

#[test]
fn empty_proposal_approves_default_eip155_namespace() {
    let negotiated = negotiate_walletconnect_namespaces(
        &BTreeMap::new(),
        &BTreeMap::new(),
        &supported_chains(&[1, 137]),
        address!("1111111111111111111111111111111111111111"),
        PublicAccountSource::Imported,
    )
    .unwrap();
    let approved = negotiated
        .approved_namespaces
        .get("eip155")
        .expect("default eip155 namespace");

    assert_eq!(approved.chains, vec!["eip155:1", "eip155:137"]);
    assert_eq!(
        approved.accounts,
        vec![
            "eip155:1:0x1111111111111111111111111111111111111111",
            "eip155:137:0x1111111111111111111111111111111111111111",
        ]
    );
    assert!(approved.methods.is_empty());
    assert!(approved.events.is_empty());
}

#[test]
fn event_only_required_namespace_is_approved() {
    let mut required = BTreeMap::new();
    required.insert(
        "eip155".to_owned(),
        namespace(&["eip155:1"], &[], &["chainChanged"]),
    );

    let negotiated = negotiate_walletconnect_namespaces(
        &required,
        &BTreeMap::new(),
        &supported_chains(&[1]),
        address!("1111111111111111111111111111111111111111"),
        PublicAccountSource::Imported,
    )
    .unwrap();
    let approved = negotiated
        .approved_namespaces
        .get("eip155")
        .expect("approved event-only namespace");

    assert_eq!(approved.methods, Vec::<String>::new());
    assert_eq!(approved.events, vec!["chainChanged"]);
}

#[test]
fn empty_required_namespace_is_approved() {
    let mut required = BTreeMap::new();
    required.insert("eip155".to_owned(), namespace(&["eip155:1"], &[], &[]));

    let negotiated = negotiate_walletconnect_namespaces(
        &required,
        &BTreeMap::new(),
        &supported_chains(&[1]),
        address!("1111111111111111111111111111111111111111"),
        PublicAccountSource::Imported,
    )
    .unwrap();
    let approved = negotiated
        .approved_namespaces
        .get("eip155")
        .expect("approved empty namespace");

    assert_eq!(approved.chains, vec!["eip155:1"]);
    assert_eq!(
        approved.accounts,
        vec!["eip155:1:0x1111111111111111111111111111111111111111"]
    );
    assert!(approved.methods.is_empty());
    assert!(approved.events.is_empty());
}

#[test]
fn event_only_optional_namespace_is_approved() {
    let mut required = BTreeMap::new();
    required.insert(
        "eip155".to_owned(),
        namespace(&["eip155:1"], &["eth_accounts"], &[]),
    );
    let mut optional = BTreeMap::new();
    optional.insert(
        "eip155:137".to_owned(),
        namespace(&[], &[], &["chainChanged"]),
    );

    let negotiated = negotiate_walletconnect_namespaces(
        &required,
        &optional,
        &supported_chains(&[1, 137]),
        address!("1111111111111111111111111111111111111111"),
        PublicAccountSource::Imported,
    )
    .unwrap();
    let approved = negotiated
        .approved_namespaces
        .get("eip155:137")
        .expect("approved optional event-only namespace");

    assert_eq!(approved.methods, Vec::<String>::new());
    assert_eq!(approved.events, vec!["chainChanged"]);
}

#[test]
fn required_namespace_rejects_unsupported_method() {
    let mut required = BTreeMap::new();
    required.insert(
        "eip155".to_owned(),
        namespace(&["eip155:1"], &["eth_accounts", "eth_sign"], &[]),
    );

    assert!(matches!(
        negotiate_walletconnect_namespaces(
            &required,
            &BTreeMap::new(),
            &supported_chains(&[1]),
            address!("1111111111111111111111111111111111111111"),
            PublicAccountSource::Imported,
        ),
        Err(WalletConnectError::UnsatisfiedNamespaces(message)) if message.contains("eth_sign")
    ));
}

#[test]
fn optional_namespace_can_be_partially_approved() {
    let mut required = BTreeMap::new();
    required.insert(
        "eip155".to_owned(),
        namespace(&["eip155:1"], &["eth_accounts"], &[]),
    );
    let mut optional = BTreeMap::new();
    optional.insert(
        "eip155:42161".to_owned(),
        namespace(
            &["eip155:42161", "eip155:999999"],
            &["eth_sendTransaction", "eth_sign"],
            &["accountsChanged", "badEvent"],
        ),
    );

    let negotiated = negotiate_walletconnect_namespaces(
        &required,
        &optional,
        &supported_chains(&[1, 42161]),
        address!("1111111111111111111111111111111111111111"),
        PublicAccountSource::Imported,
    )
    .unwrap();
    let optional = negotiated
        .approved_namespaces
        .get("eip155:42161")
        .expect("approved optional subset");

    assert_eq!(optional.chains, vec!["eip155:42161"]);
    assert_eq!(optional.methods, vec!["eth_sendTransaction"]);
    assert_eq!(optional.events, vec!["accountsChanged"]);
    assert!(
        negotiated
            .excluded_optional
            .iter()
            .any(|item| item.item == "eth_sign")
    );
}

#[test]
fn hardware_account_rejects_required_typed_data_namespace() {
    let mut required = BTreeMap::new();
    required.insert(
        "eip155".to_owned(),
        namespace(&["eip155:1"], &["eth_signTypedData_v4"], &[]),
    );

    assert!(matches!(
        negotiate_walletconnect_namespaces(
            &required,
            &BTreeMap::new(),
            &supported_chains(&[1]),
            address!("1111111111111111111111111111111111111111"),
            PublicAccountSource::HardwareDerived,
        ),
        Err(WalletConnectError::UnsatisfiedNamespaces(message))
            if message.contains("eth_signTypedData_v4") && message.contains("hardware")
    ));
}

#[cfg(not(feature = "hardware"))]
#[test]
fn default_build_hardware_account_rejects_required_signing_namespaces() {
    for method in ["personal_sign", "eth_sendTransaction"] {
        let mut required = BTreeMap::new();
        required.insert(
            "eip155".to_owned(),
            namespace(&["eip155:1"], &[method], &[]),
        );

        assert!(matches!(
            negotiate_walletconnect_namespaces(
                &required,
                &BTreeMap::new(),
                &supported_chains(&[1]),
                address!("1111111111111111111111111111111111111111"),
                PublicAccountSource::HardwareDerived,
            ),
            Err(WalletConnectError::UnsatisfiedNamespaces(message))
                if message.contains(method) && message.contains("hardware")
        ));
    }
}

#[cfg(not(feature = "hardware"))]
#[test]
fn default_build_hardware_account_excludes_optional_signing_methods() {
    let mut required = BTreeMap::new();
    required.insert(
        "eip155".to_owned(),
        namespace(&["eip155:1"], &["eth_accounts"], &[]),
    );
    let mut optional = BTreeMap::new();
    optional.insert(
        "eip155:1".to_owned(),
        namespace(&[], &["personal_sign", "eth_sendTransaction"], &[]),
    );

    let negotiated = negotiate_walletconnect_namespaces(
        &required,
        &optional,
        &supported_chains(&[1]),
        address!("1111111111111111111111111111111111111111"),
        PublicAccountSource::HardwareDerived,
    )
    .unwrap();

    assert!(negotiated.approved_namespaces.values().all(|namespace| {
        namespace
            .methods
            .iter()
            .all(|method| method != "personal_sign" && method != "eth_sendTransaction")
    }));
}

#[test]
fn optional_caip2_namespace_method_grant_is_honored_for_same_chain() {
    let mut required = BTreeMap::new();
    required.insert(
        "eip155".to_owned(),
        namespace(&["eip155:1"], &["eth_accounts"], &[]),
    );
    let mut proposal = test_proposal(required);
    proposal.optional_namespaces.insert(
        "eip155:1".to_owned(),
        namespace(&[], &["eth_sendTransaction"], &[]),
    );
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
        "optional-method-session",
        NOW,
    )
    .unwrap();
    let resolution = WalletConnectSessionAccountResolution::Usable(account.clone());
    let request = parse_walletconnect_session_request(
        21,
        "eth_sendTransaction",
        &json!([{ "from": account.address.to_string() }]),
    )
    .unwrap();

    let validation = validate_walletconnect_session_request(
        &approval.session,
        &resolution,
        &approval.session.session_topic,
        21,
        "eip155:1",
        request,
        Some(NOW + 300),
        NOW,
    )
    .unwrap();

    assert_eq!(validation.chain_id, "eip155:1");
    assert!(validation.approval_item.is_some());
}

#[test]
fn optional_generic_namespace_method_does_not_leak_to_required_chain() {
    let mut required = BTreeMap::new();
    required.insert(
        "eip155".to_owned(),
        namespace(&["eip155:1"], &["eth_accounts"], &[]),
    );
    let mut proposal = test_proposal(required);
    proposal.optional_namespaces.insert(
        "eip155".to_owned(),
        namespace(&["eip155:137"], &["eth_sendTransaction"], &[]),
    );
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
        &supported_chains(&[1, 137]),
        "optional-generic-method-session",
        NOW,
    )
    .unwrap();
    let resolution = WalletConnectSessionAccountResolution::Usable(account.clone());
    let request = parse_walletconnect_session_request(
        26,
        "eth_sendTransaction",
        &json!([{ "from": account.address.to_string() }]),
    )
    .unwrap();

    assert!(matches!(
        validate_walletconnect_session_request(
            &approval.session,
            &resolution,
            &approval.session.session_topic,
            26,
            "eip155:1",
            request.clone(),
            Some(NOW + 300),
            NOW,
        ),
        Err(WalletConnectError::UnsupportedMethod(method)) if method == "eth_sendTransaction"
    ));

    let validation = validate_walletconnect_session_request(
        &approval.session,
        &resolution,
        &approval.session.session_topic,
        27,
        "eip155:137",
        request,
        Some(NOW + 300),
        NOW,
    )
    .unwrap();
    assert!(validation.approval_item.is_some());
}

#[test]
fn approval_builds_session_settlement_and_proposal_success() {
    let mut required = BTreeMap::new();
    required.insert(
        "eip155".to_owned(),
        namespace(&["eip155:1"], &["eth_accounts"], &["accountsChanged"]),
    );
    let proposal = test_proposal(required);
    let relay_identity = WalletConnectRelayIdentity {
        signing_key: [8u8; 32],
        client_id: "relay-client".to_owned(),
    };
    let account = test_public_account(PublicAccountScope::PrivateWallet {
        wallet_uuid: "private-wallet".to_owned(),
    });

    let approval = approve_walletconnect_session(
        &proposal,
        &[1u8; 32],
        &relay_identity,
        &account,
        &supported_chains(&[1]),
        "approved-session",
        NOW,
    )
    .unwrap();

    assert_eq!(approval.session.session_uuid, "approved-session");
    assert_eq!(approval.session.expiry_timestamp, NOW + 604_800);
    assert_eq!(
        approval.session.owning_private_wallet_uuid.as_deref(),
        Some("private-wallet")
    );
    assert!(matches!(
        &approval.relay_steps[0],
        WalletConnectRelayStep::FetchMessages { topic } if topic == &approval.session.session_topic
    ));
    assert!(matches!(
        &approval.relay_steps[1],
        WalletConnectRelayStep::Subscribe { topic } if topic == &approval.session.session_topic
    ));
    let proposal_result = approval
        .approval_messages
        .proposal_response
        .result
        .as_ref()
        .expect("proposal success");
    assert!(proposal_result.get("responderPublicKey").is_some());
}

#[test]
fn expired_proposal_rejects_approval_and_builds_error_response() {
    let mut proposal = test_proposal(BTreeMap::new());
    proposal.expiry_timestamp = NOW - 1;
    let relay_identity = WalletConnectRelayIdentity {
        signing_key: [8u8; 32],
        client_id: "relay-client".to_owned(),
    };
    let account = test_public_account(PublicAccountScope::Global);

    assert!(matches!(
        approve_walletconnect_session(
            &proposal,
            &[1u8; 32],
            &relay_identity,
            &account,
            &supported_chains(&[1]),
            "expired-session",
            NOW,
        ),
        Err(WalletConnectError::ExpiredUri)
    ));

    let rejection = reject_walletconnect_session_proposal(
        proposal.id,
        WalletConnectProposalRejectionReason::Expired,
    );
    assert_eq!(rejection.error.unwrap().code, 8_000);
}

#[test]
fn all_zero_proposer_key_rejects_session_approval() {
    let mut proposal = test_proposal(BTreeMap::new());
    proposal.proposer_public_key = "00".repeat(32);
    let relay_identity = WalletConnectRelayIdentity {
        signing_key: [8u8; 32],
        client_id: "relay-client".to_owned(),
    };
    let account = test_public_account(PublicAccountScope::Global);

    assert!(matches!(
        approve_walletconnect_session(
            &proposal,
            &[1u8; 32],
            &relay_identity,
            &account,
            &supported_chains(&[1]),
            "low-order-session",
            NOW,
        ),
        Err(WalletConnectError::Crypto)
    ));
}

#[test]
fn relay_lifecycle_pauses_without_unsubscribing_and_terminal_unsubscribes() {
    let mut lifecycle = WalletConnectRelayLifecycle::default();
    lifecycle.add_subscribed_topic("session-topic", "sub-1", true);

    assert!(lifecycle.local_processing_active());
    assert!(lifecycle.pause_for_lock_or_shutdown().is_empty());
    assert!(!lifecycle.local_processing_active());

    lifecycle.resume_after_unlock();
    assert!(lifecycle.local_processing_active());

    let unsubscribe = lifecycle
        .terminal_end(
            "session-topic",
            WalletConnectTerminalLifecycleEnd::Disconnect,
        )
        .expect("unsubscribe on terminal end");
    assert!(matches!(
        unsubscribe,
        WalletConnectRelayRpc::Unsubscribe { topic, id }
            if topic == "session-topic" && id == "sub-1"
    ));
    assert!(!lifecycle.local_processing_active());
}

#[test]
fn restored_session_steps_fetch_before_subscribe() {
    let steps = WalletConnectRelayLifecycle::restored_session_steps("session-topic");

    assert!(matches!(
        &steps[0],
        WalletConnectRelayStep::FetchMessages { topic } if topic == "session-topic"
    ));
    assert!(matches!(
        &steps[1],
        WalletConnectRelayStep::Subscribe { topic } if topic == "session-topic"
    ));
}

#[test]
fn reconnect_steps_fetch_and_resubscribe_active_topics() {
    let mut lifecycle = WalletConnectRelayLifecycle::default();
    lifecycle.add_subscribed_topic("session-topic", "sub-1", true);
    let steps = lifecycle.reconnect_steps();

    assert!(matches!(
        &steps[0],
        WalletConnectRelayStep::FetchMessages { topic } if topic == "session-topic"
    ));
    assert!(matches!(
        &steps[1],
        WalletConnectRelayStep::Subscribe { topic } if topic == "session-topic"
    ));
}

#[test]
fn disconnect_plan_sends_session_delete_and_unsubscribes_when_possible() {
    let mut required = BTreeMap::new();
    required.insert(
        "eip155".to_owned(),
        namespace(&["eip155:1"], &["eth_accounts"], &["accountsChanged"]),
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
        "disconnect-session",
        NOW,
    )
    .unwrap();

    let plan = build_walletconnect_disconnect_plan(&approval.session, 99, Some("sub-1")).unwrap();

    assert_eq!(plan.delete_request.method, "wc_sessionDelete");
    assert_eq!(plan.relay_steps.len(), 2);
    assert!(matches!(
        &plan.relay_steps[0],
        WalletConnectRelayStep::Publish(_)
    ));
    let delete = plan.relay_steps[0].rpc().request(100);
    assert_eq!(delete.params["ttl"], json!(86_400));
    assert_eq!(delete.params["tag"], json!(1112));
    assert!(serde_json::from_str::<Value>(delete.params["message"].as_str().unwrap()).is_err());
    let delete_message = decode_encrypted_json(
        &approval.session.keys.sym_key,
        delete.params["message"].as_str().unwrap(),
    );
    assert_eq!(delete_message["method"], "wc_sessionDelete");
    assert!(matches!(
        &plan.relay_steps[1],
        WalletConnectRelayStep::Unsubscribe { topic, id }
            if topic == &approval.session.session_topic && id == "sub-1"
    ));
}

fn approved_request_session(
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

#[test]
fn parses_supported_requests_and_rejects_unsafe_methods() {
    assert!(matches!(
        parse_walletconnect_session_request(1, "eth_accounts", &json!([])).unwrap(),
        WalletConnectParsedRequest::EthAccounts
    ));
    assert!(matches!(
        parse_walletconnect_session_request(
            2,
            "personal_sign",
            &json!(["0x68656c6c6f", "0x1111111111111111111111111111111111111111"]),
        )
        .unwrap(),
        WalletConnectParsedRequest::PersonalSign { .. }
    ));
    assert!(matches!(
        parse_walletconnect_session_request(3, "eth_sign", &json!([])),
        Err(WalletConnectError::UnsupportedMethod(method)) if method == "eth_sign"
    ));
}

#[test]
fn rejects_malformed_personal_sign_hex_before_approval() {
    let account = address!("1111111111111111111111111111111111111111");

    assert!(matches!(
        parse_walletconnect_session_request(
            32,
            "personal_sign",
            &json!(["0xzz", account.to_string()]),
        ),
        Err(WalletConnectError::MalformedParams(message))
            if message.contains("valid hex")
    ));
    assert!(matches!(
        parse_walletconnect_session_request(
            33,
            "personal_sign",
            &json!(["0x123", account.to_string()]),
        ),
        Err(WalletConnectError::MalformedParams(message))
            if message.contains("valid hex")
    ));
    assert!(matches!(
        parse_walletconnect_session_request(
            34,
            "personal_sign",
            &json!(["plain text", account.to_string()]),
        )
        .unwrap(),
        WalletConnectParsedRequest::PersonalSign { .. }
    ));
}

#[cfg(not(feature = "hardware"))]
#[test]
fn default_build_hardware_session_request_rejects_signing_method() {
    let (session, mut account) = approved_request_session(&["personal_sign"]);
    account.source = PublicAccountSource::HardwareDerived;
    let resolution = WalletConnectSessionAccountResolution::Usable(account.clone());
    let request = parse_walletconnect_session_request(
        28,
        "personal_sign",
        &json!(["0x6869", account.address.to_string()]),
    )
    .unwrap();

    assert!(matches!(
        validate_walletconnect_session_request(
            &session,
            &resolution,
            &session.session_topic,
            28,
            "eip155:1",
            request,
            Some(NOW + 300),
            NOW,
        ),
        Err(WalletConnectError::UnsupportedMethod(method)) if method == "personal_sign"
    ));
}

#[test]
fn validates_request_permissions_and_builds_erc20_approval_item() {
    let (session, account) = approved_request_session(&[
        "eth_accounts",
        "personal_sign",
        "eth_sendTransaction",
        "eth_signTypedData_v4",
        "wallet_switchEthereumChain",
    ]);
    let resolution = WalletConnectSessionAccountResolution::Usable(account.clone());
    let approve_data = concat!(
        "0x095ea7b3",
        "0000000000000000000000002222222222222222222222222222222222222222",
        "0000000000000000000000000000000000000000000000000000000000000001"
    );
    let request = parse_walletconnect_session_request(
        10,
        "eth_sendTransaction",
        &json!([{
            "from": account.address.to_string(),
            "to": "0x3333333333333333333333333333333333333333",
            "data": approve_data,
            "chainId": "0x1"
        }]),
    )
    .unwrap();

    let validation = validate_walletconnect_session_request(
        &session,
        &resolution,
        &session.session_topic,
        10,
        "eip155:1",
        request,
        Some(NOW + 300),
        NOW,
    )
    .unwrap();
    let approval = validation.approval_item.expect("approval item");

    assert_eq!(approval.method.as_str(), "eth_sendTransaction");
    assert!(matches!(
        approval.decoded_summary,
        Some(WalletConnectErc20CallSummary::Approve { spender, amount })
            if spender == address!("2222222222222222222222222222222222222222") && amount == U256::from(1)
    ));
}

#[test]
fn accepts_session_request_expiry_with_less_than_minimum_remaining() {
    let (session, account) = approved_request_session(&["personal_sign"]);
    let resolution = WalletConnectSessionAccountResolution::Usable(account.clone());
    let request = parse_walletconnect_session_request(
        31,
        "personal_sign",
        &json!(["0x6869", account.address.to_string()]),
    )
    .unwrap();

    let validation = validate_walletconnect_session_request(
        &session,
        &resolution,
        &session.session_topic,
        31,
        "eip155:1",
        request,
        Some(NOW + 299),
        NOW,
    )
    .unwrap();
    assert!(validation.approval_item.is_some());
}

#[test]
fn rejects_session_request_expiry_when_expired_or_too_far_future() {
    let (session, account) = approved_request_session(&["personal_sign"]);
    let resolution = WalletConnectSessionAccountResolution::Usable(account.clone());
    let request = parse_walletconnect_session_request(
        31,
        "personal_sign",
        &json!(["0x6869", account.address.to_string()]),
    )
    .unwrap();

    assert!(matches!(
        validate_walletconnect_session_request(
            &session,
            &resolution,
            &session.session_topic,
            31,
            "eip155:1",
            request.clone(),
            Some(NOW),
            NOW,
        ),
        Err(WalletConnectError::ExpiredUri)
    ));
    assert!(matches!(
        validate_walletconnect_session_request(
            &session,
            &resolution,
            &session.session_topic,
            31,
            "eip155:1",
            request.clone(),
            Some(NOW + 604_801),
            NOW,
        ),
        Err(WalletConnectError::ExpiredUri)
    ));

    let validation = validate_walletconnect_session_request(
        &session,
        &resolution,
        &session.session_topic,
        31,
        "eip155:1",
        request,
        Some(NOW + 604_800),
        NOW,
    )
    .unwrap();
    assert!(validation.approval_item.is_some());
}

#[test]
fn parses_send_transaction_execution_overrides() {
    let account = address!("1111111111111111111111111111111111111111");
    let request = parse_walletconnect_session_request(
        22,
        "eth_sendTransaction",
        &json!([{
            "from": account.to_string(),
            "gas": "0x5208",
            "gasPrice": "0x3b9aca00",
            "maxFeePerGas": "0x4a817c800",
            "maxPriorityFeePerGas": "0x77359400",
            "nonce": "0x2a",
            "type": "0x1",
            "accessList": [{
                "address": "0x2222222222222222222222222222222222222222",
                "storageKeys": ["0x0000000000000000000000000000000000000000000000000000000000000003"]
            }],
        }]),
    )
    .unwrap();
    let WalletConnectParsedRequest::EthSendTransaction { transaction } = request else {
        panic!("expected eth_sendTransaction");
    };

    assert_eq!(transaction.gas, Some(U256::from(0x5208_u64)));
    assert_eq!(transaction.gas_price, Some(U256::from(1_000_000_000_u64)));
    assert_eq!(
        transaction.max_fee_per_gas,
        Some(U256::from(20_000_000_000_u64))
    );
    assert_eq!(
        transaction.max_priority_fee_per_gas,
        Some(U256::from(2_000_000_000_u64))
    );
    assert_eq!(transaction.nonce, Some(U256::from(42_u64)));
    assert_eq!(transaction.transaction_type, Some(1));
    let access_list = transaction.access_list.expect("access list");
    assert_eq!(access_list.len(), 1);
    assert_eq!(
        access_list[0].address,
        address!("2222222222222222222222222222222222222222")
    );
}

#[test]
fn wallet_switch_ethereum_chain_accepts_different_approved_target_chain() {
    let mut required = BTreeMap::new();
    required.insert(
        "eip155".to_owned(),
        namespace(
            &["eip155:1", "eip155:42161"],
            &["wallet_switchEthereumChain"],
            &["chainChanged"],
        ),
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
        &supported_chains(&[1, 42161]),
        "switch-session",
        NOW,
    )
    .unwrap();
    let resolution = WalletConnectSessionAccountResolution::Usable(account);
    let request = parse_walletconnect_session_request(
        23,
        "wallet_switchEthereumChain",
        &json!([{ "chainId": "0xa4b1" }]),
    )
    .unwrap();

    let validation = validate_walletconnect_session_request(
        &approval.session,
        &resolution,
        &approval.session.session_topic,
        23,
        "eip155:1",
        request,
        Some(NOW + 300),
        NOW,
    )
    .unwrap();

    assert!(matches!(
        validation.request,
        WalletConnectParsedRequest::WalletSwitchEthereumChain { chain_id: 42161 }
    ));
}

#[test]
fn validates_aave_style_approve_send_transaction_as_pending_request() {
    let (session, account) = approved_request_session(&["eth_sendTransaction"]);
    let resolution = WalletConnectSessionAccountResolution::Usable(account.clone());
    let approve_data = concat!(
        "0x095ea7b3",
        "0000000000000000000000002222222222222222222222222222222222222222",
        "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
    );
    let request = parse_walletconnect_session_request(
        2_526,
        "eth_sendTransaction",
        &json!([{
            "from": account.address.to_string(),
            "to": "0xdAC17F958D2ee523a2206206994597C13D831ec7",
            "data": approve_data,
            "value": "0x0"
        }]),
    )
    .unwrap();

    let validation = validate_walletconnect_session_request(
        &session,
        &resolution,
        &session.session_topic,
        2_526,
        "eip155:1",
        request,
        Some(NOW + 300),
        NOW,
    )
    .unwrap();
    let approval = validation.approval_item.expect("approval item");

    assert_eq!(approval.id, 2_526);
    assert_eq!(approval.method.as_str(), "eth_sendTransaction");
    assert_eq!(approval.chain_id, "eip155:1");
    assert_eq!(approval.account, account.address);
    assert_eq!(
        approval.raw_details["to"],
        json!("0xdAC17F958D2ee523a2206206994597C13D831ec7")
    );
    assert!(matches!(
        approval.decoded_summary,
        Some(WalletConnectErc20CallSummary::Approve { spender, amount })
            if spender == address!("2222222222222222222222222222222222222222") && amount == U256::MAX
    ));
}

#[test]
fn rejects_invalid_transaction_data_hex_before_approval() {
    let (_, account) = approved_request_session(&["eth_sendTransaction"]);

    assert!(matches!(
        parse_walletconnect_session_request(
            19,
            "eth_sendTransaction",
            &json!([{ "from": account.address.to_string(), "data": "0xzz" }]),
        ),
        Err(WalletConnectError::MalformedParams(message)) if message.contains("valid hex")
    ));
    assert!(matches!(
        parse_walletconnect_session_request(
            20,
            "eth_sendTransaction",
            &json!([{ "from": account.address.to_string(), "input": "0x123" }]),
        ),
        Err(WalletConnectError::MalformedParams(message)) if message.contains("valid hex")
    ));
}

#[test]
fn rejects_transaction_and_typed_data_chain_mismatches_before_approval() {
    let (session, account) =
        approved_request_session(&["eth_sendTransaction", "eth_signTypedData_v4"]);
    let resolution = WalletConnectSessionAccountResolution::Usable(account.clone());

    let tx_request = parse_walletconnect_session_request(
        11,
        "eth_sendTransaction",
        &json!([{ "from": account.address.to_string(), "chainId": "0xa" }]),
    )
    .unwrap();
    assert!(matches!(
        validate_walletconnect_session_request(
            &session,
            &resolution,
            &session.session_topic,
            11,
            "eip155:1",
            tx_request,
            Some(NOW + 300),
            NOW,
        ),
        Err(WalletConnectError::Relay(message)) if message.contains("transaction chainId")
    ));

    let typed_request = parse_walletconnect_session_request(
        12,
        "eth_signTypedData_v4",
        &json!([
            account.address.to_string(),
            typed_data_payload(json!("0xa"))
        ]),
    )
    .unwrap();
    assert!(matches!(
        validate_walletconnect_session_request(
            &session,
            &resolution,
            &session.session_topic,
            12,
            "eip155:1",
            typed_request,
            Some(NOW + 300),
            NOW,
        ),
        Err(WalletConnectError::Relay(message)) if message.contains("typed-data")
    ));

    let oversized_request = parse_walletconnect_session_request(
        24,
        "eth_signTypedData_v4",
        &json!([
            account.address.to_string(),
            typed_data_payload(json!(
                "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
            ))
        ]),
    )
    .unwrap();
    assert!(matches!(
        validate_walletconnect_session_request(
            &session,
            &resolution,
            &session.session_topic,
            24,
            "eip155:1",
            oversized_request,
            Some(NOW + 300),
            NOW,
        ),
        Err(WalletConnectError::Relay(message)) if message.contains("typed-data")
    ));
}

#[test]
fn rejects_malformed_typed_data_domain_chain_id_before_approval() {
    let (_, account) = approved_request_session(&["eth_signTypedData_v4"]);

    assert!(matches!(
        parse_walletconnect_session_request(
            25,
            "eth_signTypedData_v4",
            &json!([
                account.address.to_string(),
                typed_data_payload(json!("0x10000000000000000000000000000000000000000000000000000000000000000"))
            ]),
        ),
        Err(WalletConnectError::MalformedParams(message)) if message.contains("domain.chainId")
    ));
}

#[test]
fn rejects_malformed_typed_data_payload_before_approval() {
    let (_, account) = approved_request_session(&["eth_signTypedData_v4"]);

    assert!(matches!(
        parse_walletconnect_session_request(
            29,
            "eth_signTypedData_v4",
            &json!([account.address.to_string(), {}]),
        ),
        Err(WalletConnectError::MalformedParams(message))
            if message.contains("invalid EIP-712")
    ));

    assert!(matches!(
        parse_walletconnect_session_request(
            30,
            "eth_signTypedData_v4",
            &json!([
                account.address.to_string(),
                {
                    "types": {
                        "EIP712Domain": [],
                        "Message": [{ "name": "contents", "type": "string" }]
                    },
                    "domain": {},
                    "message": { "contents": "hello" }
                }
            ]),
        ),
        Err(WalletConnectError::MalformedParams(message))
            if message.contains("invalid EIP-712")
    ));
}

#[test]
fn pending_queue_removes_expired_requests() {
    let mut queue = WalletConnectPendingRequestQueue::default();
    let (session, account) = approved_request_session(&["personal_sign"]);
    let resolution = WalletConnectSessionAccountResolution::Usable(account.clone());
    let request = parse_walletconnect_session_request(
        13,
        "personal_sign",
        &json!(["0x6869", account.address.to_string()]),
    )
    .unwrap();
    let validation = validate_walletconnect_session_request(
        &session,
        &resolution,
        &session.session_topic,
        13,
        "eip155:1",
        request,
        Some(NOW + 300),
        NOW,
    )
    .unwrap();

    queue.insert(validation.approval_item.expect("approval item"));
    assert!(queue.get(13).is_some());
    let expired = queue.remove_expired(NOW + 301);

    assert_eq!(expired.len(), 1);
    assert!(queue.get(13).is_none());
}

#[test]
fn session_event_builder_checks_approved_event_and_chain_payload() {
    let (session, _) = approved_request_session(&["eth_accounts"]);

    let event =
        build_walletconnect_session_event(&session, 14, "eip155:1", "chainChanged", json!(1))
            .unwrap();

    assert_eq!(event.method, "wc_sessionEvent");
    assert_eq!(event.params["chainId"], "eip155:1");
    assert_eq!(event.params["event"]["data"], 1);
    assert!(matches!(
        build_walletconnect_session_event(&session, 15, "eip155:1", "badEvent", json!(null)),
        Err(WalletConnectError::UnsupportedEvent(event)) if event == "badEvent"
    ));
}

#[test]
fn lifecycle_delete_ping_and_error_mapping_are_protocol_compatible() {
    assert!(matches!(
        handle_walletconnect_lifecycle_request(16, "wc_sessionDelete"),
        WalletConnectLifecycleRequestOutcome::Delete { response }
            if response.result == Some(json!(true))
    ));
    assert!(matches!(
        handle_walletconnect_lifecycle_request(17, "wc_sessionPing"),
        WalletConnectLifecycleRequestOutcome::Ping { response }
            if response.result == Some(json!(true))
    ));

    let expired = build_walletconnect_jsonrpc_error(
        18,
        WalletConnectRequestErrorKind::ExpiredRequest,
        "expired",
    );
    assert_eq!(expired.error.unwrap().code, 8_000);
}
