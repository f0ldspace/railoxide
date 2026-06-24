use super::fixtures::*;
use super::*;

#[test]
fn active_sessions_exclude_expired_sessions() {
    let now = current_unix_seconds();
    let mut active = test_walletconnect_session("active-session-topic");
    active.expiry_timestamp = now + 300;
    let mut expired = test_walletconnect_session("expired-session-topic");
    expired.session_uuid = "expired-session".to_owned();
    expired.expiry_timestamp = now;

    let sessions = walletconnect_active_sessions(&[active, expired], &BTreeMap::new());

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_topic, "active-session-topic");
}

#[test]
fn management_visibility_includes_terminal_sessions() {
    let mut invalid = test_walletconnect_session("invalid-session-topic");
    invalid.lifecycle_state = WalletConnectSessionLifecycleState::Invalid;
    let mut expired = test_walletconnect_session("expired-session-topic");
    expired.lifecycle_state = WalletConnectSessionLifecycleState::Expired;
    expired.expiry_timestamp = current_unix_seconds().saturating_sub(1);

    assert!(walletconnect_session_visible_in_management(&invalid));
    assert!(walletconnect_session_visible_in_management(&expired));
    assert!(!walletconnect_session_relay_processable(
        &expired,
        current_unix_seconds()
    ));
}

#[test]
fn paused_session_labels_explain_wallet_switch() {
    let mut session = test_walletconnect_session("paused-session-topic");
    session.selected_public_account_uuid = "a1cbbbedf2ca1-extra-uuid".to_owned();
    session.lifecycle_state = WalletConnectSessionLifecycleState::TemporarilyPaused;

    assert_eq!(
        walletconnect_lifecycle_label(session.lifecycle_state),
        "Paused: switch to owning wallet"
    );
    assert_eq!(
        walletconnect_unresolved_public_account_label(&session),
        "Account from another wallet"
    );

    session.lifecycle_state = WalletConnectSessionLifecycleState::Invalid;
    assert_eq!(
        walletconnect_unresolved_public_account_label(&session),
        "a1cbbbedf2ca"
    );
}

#[test]
fn relay_processing_sessions_include_paused_scoped_sessions() {
    let now = current_unix_seconds();
    let mut active = test_walletconnect_session("active-session-topic");
    active.expiry_timestamp = now + 300;
    let mut paused = test_walletconnect_session("paused-session-topic");
    paused.session_uuid = "paused-session".to_owned();
    paused.lifecycle_state = WalletConnectSessionLifecycleState::TemporarilyPaused;
    paused.expiry_timestamp = now + 300;
    let mut invalid = test_walletconnect_session("invalid-session-topic");
    invalid.session_uuid = "invalid-session".to_owned();
    invalid.lifecycle_state = WalletConnectSessionLifecycleState::Invalid;
    invalid.expiry_timestamp = now + 300;

    let sessions = walletconnect_active_sessions(&[active, paused, invalid], &BTreeMap::new());

    assert_eq!(sessions.len(), 2);
    assert!(
        sessions
            .iter()
            .any(|session| session.session_topic == "active-session-topic")
    );
    assert!(
        sessions
            .iter()
            .any(|session| session.session_topic == "paused-session-topic")
    );
}

#[test]
fn paused_scoped_session_request_rejects_unauthorized() {
    let (root_dir, store) = walletconnect_test_store();
    let first_session = import_test_wallet(&store, "wc-paused-a", "WC Paused A");
    let second_session = import_test_wallet(&store, "wc-paused-b", "WC Paused B");
    let scoped = store
        .import_public_account(
            TEST_PASSWORD,
            &first_session,
            TEST_IMPORTED_PRIVATE_KEY,
            Some("Scoped WC"),
            false,
        )
        .expect("import scoped public account");
    let mut session = test_walletconnect_session("paused-session-topic");
    session.session_uuid = "paused-session".to_owned();
    session.selected_public_account_uuid = scoped.public_account_uuid.clone();
    session.selected_public_account_scope = scoped.scope;
    session.owning_private_wallet_uuid = Some(first_session.wallet_id().to_owned());
    session.lifecycle_state = WalletConnectSessionLifecycleState::TemporarilyPaused;
    session.expiry_timestamp = current_unix_seconds() + 300;
    let request = WalletConnectJsonRpcRequest::new(
        77,
        "wc_sessionRequest",
        json!({
            "chainId": "eip155:1",
            "request": {
                "method": "eth_accounts",
                "params": []
            }
        }),
    );
    let plaintext = serde_json::to_vec(&request).expect("request json");
    let message = encode_walletconnect_message(&session.keys.sym_key, &plaintext)
        .expect("encode request")
        .to_base64();
    let relay_message = WalletConnectRelayMessage {
        topic: session.session_topic.clone(),
        message,
    };

    let outcome = process_walletconnect_session_message(
        &store,
        &second_session,
        &session,
        &relay_message,
        &BTreeSet::from([1]),
        current_unix_seconds(),
    )
    .expect("process request");

    let SessionMessageOutcome::Respond { response, .. } = outcome else {
        panic!("expected unauthorized response");
    };
    assert_eq!(
        response.error.expect("error response").code,
        WalletConnectRequestErrorKind::Unauthorized.code()
    );

    drop(store);
    fs::remove_dir_all(root_dir).expect("remove temp db dir");
}

#[test]
fn lifecycle_responses_use_method_specific_irn_metadata() {
    let (root_dir, store) = walletconnect_test_store();
    let view_session = import_test_wallet(&store, "wc-lifecycle-tags", "WC Lifecycle Tags");
    let mut session = test_walletconnect_session("lifecycle-session-topic");
    session.session_uuid = "lifecycle-session".to_owned();
    store
        .store_walletconnect_session(&view_session, &session)
        .expect("store lifecycle session");

    let delete = WalletConnectJsonRpcRequest::new(
        91,
        "wc_sessionDelete",
        json!({ "code": 6000, "message": "User disconnected" }),
    );
    let delete_message = test_walletconnect_relay_message(&session, &delete);
    let delete_outcome = process_walletconnect_session_message(
        &store,
        &view_session,
        &session,
        &delete_message,
        &BTreeSet::from([1]),
        current_unix_seconds(),
    )
    .expect("process delete");

    let SessionMessageOutcome::Respond {
        response_ttl,
        response_tag,
        removed_session,
        ..
    } = delete_outcome
    else {
        panic!("expected delete response");
    };
    assert_eq!(response_ttl, WALLETCONNECT_SESSION_DELETE_TTL_SECS);
    assert_eq!(response_tag, WC_SESSION_DELETE_RESPONSE_TAG);
    assert_eq!(removed_session.as_deref(), Some("lifecycle-session"));

    let ping = WalletConnectJsonRpcRequest::new(92, "wc_sessionPing", json!({}));
    let ping_message = test_walletconnect_relay_message(&session, &ping);
    let ping_outcome = process_walletconnect_session_message(
        &store,
        &view_session,
        &session,
        &ping_message,
        &BTreeSet::from([1]),
        current_unix_seconds(),
    )
    .expect("process ping");

    let SessionMessageOutcome::Respond {
        response_ttl,
        response_tag,
        removed_session,
        ..
    } = ping_outcome
    else {
        panic!("expected ping response");
    };
    assert_eq!(response_ttl, WALLETCONNECT_SESSION_PING_TTL_SECS);
    assert_eq!(response_tag, WC_SESSION_PING_RESPONSE_TAG);
    assert!(removed_session.is_none());

    drop(store);
    fs::remove_dir_all(root_dir).expect("remove temp db dir");
}
