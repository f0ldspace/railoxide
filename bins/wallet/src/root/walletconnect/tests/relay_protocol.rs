use super::fixtures::*;
use super::*;

#[test]
fn relay_messages_accept_walletconnect_metadata_and_has_more() {
    let value = json!({
        "messages": [{
            "topic": "session-topic",
            "message": "encrypted-approve-payload",
            "tag": 1108,
            "publishedAt": 1700000000u64,
            "attestation": {
                "origin": "https://app.aave.com"
            }
        }],
        "hasMore": true
    });

    let messages = relay_messages_from_value("fallback-topic", &value);

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].topic, "session-topic");
    assert_eq!(messages[0].message, "encrypted-approve-payload");
    assert!(relay_fetch_response_has_more(&value));
}

#[test]
fn fetch_page_limit_requires_complete_mailbox_drain() {
    assert!(!walletconnect_fetch_page_limit_exhausted(
        WALLETCONNECT_FETCH_MAX_PAGES - 2,
        true,
    ));
    assert!(walletconnect_fetch_page_limit_exhausted(
        WALLETCONNECT_FETCH_MAX_PAGES - 1,
        true,
    ));
    assert!(!walletconnect_fetch_page_limit_exhausted(
        WALLETCONNECT_FETCH_MAX_PAGES - 1,
        false,
    ));
}

#[test]
fn relay_subscription_id_accepts_string_and_object_response_shapes() {
    assert_eq!(
        relay_subscription_id_from_value(&json!("sub-string")),
        Some("sub-string".to_owned())
    );
    assert_eq!(
        relay_subscription_id_from_value(&json!({ "subscriptionId": "sub-object" })),
        Some("sub-object".to_owned())
    );
}

#[test]
fn session_jsonrpc_response_without_method_is_ignored() {
    let session = test_walletconnect_session("session-topic");
    let response = WalletConnectJsonRpcResponse {
        id: 99,
        jsonrpc: "2.0".to_owned(),
        result: Some(json!(true)),
        error: None,
    };
    let plaintext = serde_json::to_vec(&response).unwrap();
    let encoded = encode_walletconnect_message(&session.keys.sym_key, &plaintext)
        .unwrap()
        .to_base64();

    assert_eq!(
        decode_session_jsonrpc_message(&session, &encoded).unwrap(),
        DecodedSessionJsonRpcMessage::Response,
    );
}

#[test]
fn relay_worker_errors_are_classified_as_transient() {
    assert!(walletconnect_is_transient_relay_error(
        "WalletConnect relay error: websocket upgrade failed: SocksConnect(V5(Command(GeneralServerFailure)))"
    ));
    assert!(walletconnect_is_transient_relay_error(
        "WalletConnect relay error: relay response timed out"
    ));
    assert!(!walletconnect_is_transient_relay_error(
        "Could not decode WalletConnect proposal: missing field method"
    ));
}

#[test]
fn relay_request_not_sent_error_is_detected() {
    assert!(walletconnect_relay_request_was_not_sent(
        "WalletConnect relay is reconnecting; request was not sent"
    ));
    assert!(!walletconnect_relay_request_was_not_sent(
        "WalletConnect relay error: relay response timed out"
    ));
}

#[test]
fn relay_request_id_seed_includes_millisecond_time_and_entropy_digits() {
    let first = walletconnect_request_id_seed();
    let second = walletconnect_request_id_seed();

    assert_ne!(first, second);
    assert!(second > first);
    assert!(first >= WALLETCONNECT_RELAY_ID_ENTROPY_FACTOR);
    assert!(first.to_string().len() >= 19);
}
