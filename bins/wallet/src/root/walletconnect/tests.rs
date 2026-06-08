use super::{helpers::*, relay::*, render::*, requests::*, *};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use wallet_ops::WalletConnectNamespaceProposal;
use wallet_ops::hardware::{
    HardwareDerivationDescriptor, HardwareDeviceKind, HardwarePublicAccountDescriptor,
    HardwareViewAccessKey, HardwareWalletSyncIntent, parse_bip32_path,
};
use wallet_ops::vault::{
    KdfParams, PublicAccountScope, WalletConnectApprovedNamespace, WalletConnectSessionKeys,
    WalletSource,
};

const TEST_PASSWORD: &str = "correct horse battery staple";
const TEST_MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const TEST_IMPORTED_PRIVATE_KEY: &str =
    "0x59c6995e998f97a5a0044966f0945387e7d5e4a4dbd4b3f1b530b87d9b4a5c2f";

fn test_kdf() -> KdfParams {
    KdfParams::new(1024, 1, 1)
}

fn temp_db_root() -> PathBuf {
    let dir = std::env::temp_dir().join("railoxide-walletconnect-root-tests");
    fs::create_dir_all(&dir).expect("create temp db dir");
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let counter = WALLETCONNECT_RELAY_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    dir.join(format!("db-{pid}-{nanos}-{counter}"))
}

fn walletconnect_test_store() -> (PathBuf, DesktopVaultStore) {
    let root_dir = temp_db_root();
    let store = DesktopVaultStore::open(root_dir.clone()).expect("open store");
    store
        .create_vault_with_params(TEST_PASSWORD, test_kdf())
        .expect("create vault");
    (root_dir, store)
}

fn import_test_wallet(
    store: &DesktopVaultStore,
    wallet_id: &str,
    label: &str,
) -> DesktopViewSession {
    let metadata = store
        .new_wallet_metadata(TEST_PASSWORD, wallet_id, 0, WalletSource::Imported, label)
        .expect("wallet metadata");
    store
        .import_wallet_mnemonic_with_metadata(
            TEST_PASSWORD,
            wallet_id,
            0,
            "english",
            TEST_MNEMONIC,
            &metadata,
        )
        .expect("import wallet");
    store
        .load_view_session(TEST_PASSWORD, wallet_id)
        .expect("load wallet")
}

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
fn relay_worker_sync_steps_fetch_before_subscribe_topics() {
    let topics = BTreeSet::from(["pairing-topic".to_owned(), "session-topic".to_owned()]);

    let steps = walletconnect_relay_sync_steps(&topics);

    assert!(matches!(
        &steps[0],
        WalletConnectRelayStep::FetchMessages { topic } if topic == "pairing-topic"
    ));
    assert!(matches!(
        &steps[1],
        WalletConnectRelayStep::Subscribe { topic } if topic == "pairing-topic"
    ));
    assert!(matches!(
        &steps[2],
        WalletConnectRelayStep::FetchMessages { topic } if topic == "session-topic"
    ));
    assert!(matches!(
        &steps[3],
        WalletConnectRelayStep::Subscribe { topic } if topic == "session-topic"
    ));
}

#[test]
fn relay_worker_resync_unsubscribes_removed_expired_topic() {
    let current_topics = BTreeSet::from([
        "active-session-topic".to_owned(),
        "expired-session-topic".to_owned(),
    ]);
    let next_topics = BTreeSet::from(["active-session-topic".to_owned()]);
    let subscriptions = BTreeMap::from([
        ("active-session-topic".to_owned(), "sub-active".to_owned()),
        ("expired-session-topic".to_owned(), "sub-expired".to_owned()),
    ]);

    let steps =
        walletconnect_relay_topic_resync_steps(&current_topics, &next_topics, &subscriptions);

    assert!(matches!(
        &steps[0],
        WalletConnectRelayStep::Unsubscribe { topic, id }
            if topic == "expired-session-topic" && id == "sub-expired"
    ));
    assert!(matches!(
        &steps[1],
        WalletConnectRelayStep::FetchMessages { topic } if topic == "active-session-topic"
    ));
    assert!(matches!(
        &steps[2],
        WalletConnectRelayStep::Subscribe { topic } if topic == "active-session-topic"
    ));
}

#[test]
fn rejected_pairing_resync_unsubscribes_pairing_topic() {
    let current_topics = BTreeSet::from([
        "pairing-topic".to_owned(),
        "active-session-topic".to_owned(),
    ]);
    let next_topics = BTreeSet::from(["active-session-topic".to_owned()]);
    let subscriptions = BTreeMap::from([
        ("pairing-topic".to_owned(), "sub-pairing".to_owned()),
        ("active-session-topic".to_owned(), "sub-session".to_owned()),
    ]);

    let steps =
        walletconnect_relay_topic_resync_steps(&current_topics, &next_topics, &subscriptions);

    assert!(matches!(
        &steps[0],
        WalletConnectRelayStep::Unsubscribe { topic, id }
            if topic == "pairing-topic" && id == "sub-pairing"
    ));
    assert!(steps.iter().all(|step| !matches!(
        step,
        WalletConnectRelayStep::FetchMessages { topic }
            | WalletConnectRelayStep::Subscribe { topic }
            if topic == "pairing-topic"
    )));
}

#[test]
fn approval_handoff_session_is_relay_processing_target() {
    let session = test_walletconnect_session("approved-session-topic");
    let handoff_sessions = BTreeMap::from([(session.session_topic.clone(), session)]);

    let sessions =
        walletconnect_active_sessions_for_relay_client(&[], &handoff_sessions, "relay-client");

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_topic, "approved-session-topic");
    assert_eq!(
        walletconnect_relay_target_topics(&[], &sessions),
        vec!["approved-session-topic".to_owned()]
    );
}

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
    session.selected_public_account_scope = scoped.scope.clone();
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
fn terminal_unsubscribe_steps_cover_last_known_subscription() {
    let subscriptions = BTreeMap::from([("session-topic".to_owned(), "sub-1".to_owned())]);

    let steps = walletconnect_terminal_unsubscribe_steps(&subscriptions);

    assert_eq!(steps.len(), 1);
    assert!(matches!(
        &steps[0],
        WalletConnectRelayStep::Unsubscribe { topic, id }
            if topic == "session-topic" && id == "sub-1"
    ));
}

#[test]
fn stale_relay_worker_stop_unsubscribes_when_another_client_remains_active() {
    let (stale_tx, mut stale_rx) = mpsc::unbounded_channel();
    let (active_tx, mut active_rx) = mpsc::unbounded_channel();
    let mut relay_workers = BTreeMap::from([
        (
            "client-a".to_owned(),
            WalletConnectRelayWorkerHandle {
                worker_id: 1,
                project_id: "project-a".to_owned(),
                command_tx: stale_tx,
            },
        ),
        (
            "client-b".to_owned(),
            WalletConnectRelayWorkerHandle {
                worker_id: 2,
                project_id: "project-a".to_owned(),
                command_tx: active_tx,
            },
        ),
    ]);
    let active_client_ids = BTreeSet::from(["client-b".to_owned()]);

    stop_stale_walletconnect_relay_workers(&mut relay_workers, &active_client_ids, true);

    assert!(!relay_workers.contains_key("client-a"));
    assert!(relay_workers.contains_key("client-b"));
    assert!(matches!(
        stale_rx.try_recv().expect("stale worker stop command"),
        WalletConnectRelayWorkerCommand::StopAfterUnsubscribe
    ));
    assert!(active_rx.try_recv().is_err());
}

#[tokio::test]
async fn expired_pairing_relay_output_removes_pairing_without_decoding() {
    let (root_dir, store) = walletconnect_test_store();
    let view_session = import_test_wallet(&store, "wc-expired-pairing", "WC Expired Pairing");
    let expired_topic = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
    let pairing = WalletConnectPairingUri::parse_with_now(
            &format!(
                "wc:{expired_topic}@2?relay-protocol=irn&symKey=000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f&expiryTimestamp=10"
            ),
            1,
        )
        .expect("pairing");
    let (command_tx, _command_rx) = mpsc::unbounded_channel();
    let worker = WalletConnectRelayWorkerHandle {
        worker_id: 1,
        project_id: "project-a".to_owned(),
        command_tx,
    };

    let result = process_walletconnect_relay_output(
        &worker,
        &store,
        &view_session,
        std::slice::from_ref(&pairing),
        &[],
        &BTreeSet::from([1]),
        WalletConnectRelayOutput {
            messages: vec![WalletConnectRelayMessage {
                topic: pairing.topic.clone(),
                message: "not-decodable-after-expiry".to_owned(),
            }],
            subscriptions: BTreeMap::new(),
        },
        11,
    )
    .await;

    assert!(result.proposals.is_empty());
    assert_eq!(result.removed_pairings, vec![expired_topic.to_owned()]);
    assert!(result.error.is_none());

    drop(view_session);
    drop(store);
    fs::remove_dir_all(root_dir).expect("remove temp db dir");
}

#[tokio::test]
async fn recorded_pairing_processes_fetched_proposal_without_subscription() {
    let (root_dir, store) = walletconnect_test_store();
    let view_session = import_test_wallet(&store, "wc-fetched-proposal", "WC Fetched Proposal");
    let pairing_topic = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let pairing = WalletConnectPairingUri::parse_with_now(
            &format!(
                "wc:{pairing_topic}@2?relay-protocol=irn&symKey=000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f&expiryTimestamp={}"
                , current_unix_seconds() + 300
            ),
            current_unix_seconds(),
        )
        .expect("pairing");
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
                "publicKey": alloy::hex::encode([4u8; 32]),
                "metadata": {
                    "name": "Fetched Proposal Dapp",
                    "description": "Example",
                    "url": "https://example.invalid",
                    "icons": []
                }
            },
            "relays": [{ "protocol": "irn" }],
            "expiryTimestamp": current_unix_seconds() + 60
        }
    });
    let message = encode_walletconnect_message(
        &pairing.sym_key,
        serde_json::to_string(&proposal_request)
            .expect("proposal json")
            .as_bytes(),
    )
    .expect("encode proposal")
    .to_base64();
    let (command_tx, _command_rx) = mpsc::unbounded_channel();
    let worker = WalletConnectRelayWorkerHandle {
        worker_id: 1,
        project_id: "project-a".to_owned(),
        command_tx,
    };

    let result = process_walletconnect_relay_output(
        &worker,
        &store,
        &view_session,
        std::slice::from_ref(&pairing),
        &[],
        &BTreeSet::from([1]),
        WalletConnectRelayOutput {
            messages: vec![WalletConnectRelayMessage {
                topic: pairing.topic.clone(),
                message,
            }],
            subscriptions: BTreeMap::new(),
        },
        current_unix_seconds(),
    )
    .await;

    assert_eq!(result.proposals.len(), 1);
    assert_eq!(result.proposals[0].proposal.id, 42);
    assert_eq!(
        result.proposals[0].proposal.peer_metadata.name,
        "Fetched Proposal Dapp"
    );
    assert!(result.error.is_none());

    drop(store);
    fs::remove_dir_all(root_dir).expect("remove temp db dir");
}

#[test]
fn unsupported_required_namespaces_reject_as_unsupported() {
    let account = PublicAccountMetadata {
        public_account_uuid: "public-account".to_owned(),
        address: alloy::primitives::Address::from([0x11; 20]),
        label: None,
        source: PublicAccountSource::Imported,
        scope: PublicAccountScope::Global,
        derivation_index: None,
        hardware_descriptor: None,
        status: PublicAccountStatus::Active,
        display_order: 0,
    };
    let mut required_namespaces = BTreeMap::new();
    required_namespaces.insert(
        "eip155".to_owned(),
        WalletConnectNamespaceProposal {
            chains: vec!["eip155:1".to_owned()],
            methods: vec!["eth_accounts".to_owned(), "eth_sign".to_owned()],
            events: Vec::new(),
        },
    );
    let proposal = WalletConnectSessionProposal {
        id: 1,
        pairing_topic: "pairing-topic".to_owned(),
        proposer_public_key: "00".repeat(32),
        relay_protocol: "irn".to_owned(),
        peer_metadata: WalletConnectPeerMetadata {
            name: "Unsupported Dapp".to_owned(),
            description: String::new(),
            url: "https://example.invalid".to_owned(),
            icons: Vec::new(),
        },
        required_namespaces,
        optional_namespaces: BTreeMap::new(),
        expiry_timestamp: 20,
    };

    let reason = walletconnect_proposal_rejection_reason(
        &proposal,
        Some(&account),
        None,
        &BTreeSet::from([1]),
        10,
    );

    assert_eq!(
        reason,
        WalletConnectProposalRejectionReason::UnsupportedNamespaces
    );
}

#[test]
fn hardware_typed_data_error_maps_to_unsupported_method() {
    let mut request = test_walletconnect_request("session-topic:7", Some(1_700_000_300));
    request.item.method = WalletConnectSupportedMethod::EthSignTypedDataV4;
    request.account_source = PublicAccountSource::HardwareDerived;
    let error = eyre::eyre!(
        "WalletConnect eth_signTypedData_v4 is unsupported for hardware Public accounts"
    );

    assert_eq!(
        walletconnect_request_approval_error_kind(&request, &error),
        WalletConnectRequestErrorKind::UnsupportedMethod
    );
}

#[test]
fn hardware_typed_data_recovery_mismatch_maps_to_error_response() {
    let mut request = test_walletconnect_request("session-topic:7", Some(1_700_000_300));
    request.item.method = WalletConnectSupportedMethod::EthSignTypedDataV4;
    request.account_source = PublicAccountSource::HardwareDerived;
    let error = eyre::eyre!(
        "hardware public signer address mismatch: expected 0x1111111111111111111111111111111111111111, got 0x2222222222222222222222222222222222222222"
    );

    let response = build_walletconnect_jsonrpc_error(
        request.item.id,
        walletconnect_request_approval_error_kind(&request, &error),
        format_report_chain(&error),
    );

    assert!(response.result.is_none());
    let error = response.error.expect("error response");
    assert_eq!(
        error.code,
        WalletConnectRequestErrorKind::UnsupportedMethod.code()
    );
    assert!(error.message.contains("address mismatch"));
}

#[test]
fn hardware_device_cancel_maps_to_user_rejected() {
    let request = test_walletconnect_request("session-topic:7", Some(1_700_000_300));
    let error = eyre::eyre!("Trezor ActionCancelled: user cancelled on device");

    assert!(is_walletconnect_user_rejected_error(&error));
    assert_eq!(
        walletconnect_request_approval_error_kind(&request, &error),
        WalletConnectRequestErrorKind::UserRejected
    );
}

#[test]
fn nested_walletconnect_request_expiry_is_preferred() {
    let request_params = json!({
        "chainId": "eip155:1",
        "expiryTimestamp": 1_700_000_300u64,
        "request": {
            "method": "eth_accounts",
            "expiryTimestamp": 1_700_000_010u64
        }
    });
    let request_payload = request_params.get("request").unwrap();

    let expiry =
        match walletconnect_session_request_expiry_timestamp(&request_params, request_payload) {
            Ok(expiry) => expiry,
            Err(error) => panic!("request expiry failed: {}", error.message),
        };

    assert_eq!(expiry, Some(1_700_000_010));
}

#[test]
fn approval_relay_steps_store_before_first_publish() {
    let steps = vec![
        WalletConnectRelayStep::FetchMessages {
            topic: "session-topic".to_owned(),
        },
        WalletConnectRelayStep::Subscribe {
            topic: "session-topic".to_owned(),
        },
        WalletConnectRelayStep::Publish(WalletConnectRelayRpc::Publish {
            topic: "session-topic".to_owned(),
            message: "settle".to_owned(),
            ttl: 300,
            tag: 1102,
        }),
        WalletConnectRelayStep::Publish(WalletConnectRelayRpc::Publish {
            topic: "pairing-topic".to_owned(),
            message: "proposal-response".to_owned(),
            ttl: 300,
            tag: 1101,
        }),
    ];

    let (pre_persist, post_persist) = walletconnect_split_pre_persist_relay_steps(steps);

    assert_eq!(pre_persist.len(), 2);
    assert!(
        pre_persist
            .iter()
            .all(|step| !matches!(step, WalletConnectRelayStep::Publish(_)))
    );
    assert_eq!(post_persist.len(), 2);
    assert!(
        post_persist
            .iter()
            .all(|step| matches!(step, WalletConnectRelayStep::Publish(_)))
    );
}

#[tokio::test]
async fn approval_post_persist_relay_error_removes_session() {
    let (root_dir, store) = walletconnect_test_store();
    let store = Arc::new(store);
    let view_session = Arc::new(import_test_wallet(
        store.as_ref(),
        "wc-approval-timeout",
        "WC Approval Timeout",
    ));
    let mut session = test_walletconnect_session("approval-timeout-topic");
    session.session_uuid = "approval-timeout-session".to_owned();
    let steps = vec![WalletConnectRelayStep::Publish(
        WalletConnectRelayRpc::Publish {
            topic: "approval-timeout-topic".to_owned(),
            message: "settle-or-proposal-response".to_owned(),
            ttl: 300,
            tag: 1101,
        },
    )];
    let (command_tx, mut command_rx) = mpsc::unbounded_channel();
    let worker = WalletConnectRelayWorkerHandle {
        worker_id: 1,
        project_id: "project-a".to_owned(),
        command_tx,
    };
    let task_store = Arc::clone(&store);
    let task_view_session = Arc::clone(&view_session);
    let task_session = session.clone();
    let approval = tokio::spawn(async move {
        execute_walletconnect_approval_relay_steps(
            &worker,
            task_store.as_ref(),
            task_view_session.as_ref(),
            &task_session,
            steps,
        )
        .await
    });

    let command = command_rx.recv().await.expect("pre-persist command");
    let WalletConnectRelayWorkerCommand::Execute {
        steps, response_tx, ..
    } = command
    else {
        panic!("expected pre-persist execute command");
    };
    assert!(steps.is_empty());
    assert!(
        response_tx
            .send(Ok(WalletConnectRelayOutput::default()))
            .is_ok()
    );

    let command = command_rx.recv().await.expect("post-persist command");
    let WalletConnectRelayWorkerCommand::Execute {
        steps, response_tx, ..
    } = command
    else {
        panic!("expected post-persist execute command");
    };
    assert!(matches!(
        &steps[0],
        WalletConnectRelayStep::Publish(WalletConnectRelayRpc::Publish { topic, .. })
            if topic == "approval-timeout-topic"
    ));
    assert!(
        response_tx
            .send(Err("relay response timed out".to_owned()))
            .is_ok()
    );

    let result = approval.await.expect("approval task").unwrap();

    assert_eq!(
        result.post_persist_error.as_deref(),
        Some("relay response timed out")
    );
    assert!(
        store
            .load_walletconnect_session(view_session.as_ref(), &session.session_uuid)
            .is_err()
    );

    drop(view_session);
    drop(store);
    fs::remove_dir_all(root_dir).expect("remove temp db dir");
}

#[test]
fn walletconnect_review_token_detects_replaced_request() {
    let mut reviewed = test_walletconnect_request("session-topic:7", Some(1_700_000_300));
    reviewed.review_token = 10;
    let mut replacement = test_walletconnect_request("session-topic:7", Some(1_700_000_600));
    replacement.review_token = 11;

    assert!(walletconnect_request_matches_review_token(&reviewed, 10));
    assert!(!walletconnect_request_matches_review_token(
        &replacement,
        10
    ));
}

#[test]
fn walletconnect_transaction_progress_uses_request_specific_steps() {
    let mut request = test_walletconnect_request("session-topic:7", Some(1_700_000_300));
    request.parsed = WalletConnectParsedRequest::EthSendTransaction {
        transaction: WalletConnectEvmTransaction {
            from: request.item.account,
            to: None,
            value: None,
            data: None,
            access_list: None,
            gas: None,
            gas_price: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            chain_id: None,
            nonce: None,
            transaction_type: None,
            raw: json!({}),
        },
    };

    let progress = WalletConnectApprovalProgress::new(1, &request);

    assert_eq!(progress.generation, 1);
    assert_eq!(
        progress
            .steps
            .iter()
            .map(|step| step.step)
            .collect::<Vec<_>>(),
        vec![
            WalletConnectApprovalProgressStep::PrepareRequest,
            WalletConnectApprovalProgressStep::ApproveOnDevice,
            WalletConnectApprovalProgressStep::BroadcastTransaction,
            WalletConnectApprovalProgressStep::RespondToDapp,
        ]
    );
    assert_eq!(progress.steps[0].status, PublicActionStepStatus::Pending);
    assert!(
        progress.steps[1..]
            .iter()
            .all(|step| step.status == PublicActionStepStatus::NotStarted)
    );
}

#[test]
fn walletconnect_personal_sign_progress_starts_at_device_approval() {
    let mut request = test_walletconnect_request("session-topic:7", Some(1_700_000_300));
    request.parsed = WalletConnectParsedRequest::PersonalSign {
        message: "0x68656c6c6f".to_owned(),
        account: request.item.account,
    };

    let progress = WalletConnectApprovalProgress::new(2, &request);

    assert_eq!(
        progress
            .steps
            .iter()
            .map(|step| step.step)
            .collect::<Vec<_>>(),
        vec![
            WalletConnectApprovalProgressStep::ApproveOnDevice,
            WalletConnectApprovalProgressStep::RespondToDapp,
        ]
    );
    assert_eq!(progress.steps[0].status, PublicActionStepStatus::Pending);
    assert_eq!(progress.steps[1].status, PublicActionStepStatus::NotStarted);
}

#[test]
fn walletconnect_typed_data_hardware_progress_starts_at_device_approval() {
    let mut request = test_walletconnect_request("session-topic:7", Some(1_700_000_300));
    request.account_source = PublicAccountSource::HardwareDerived;
    request.item.method = WalletConnectSupportedMethod::EthSignTypedDataV4;
    request.parsed = WalletConnectParsedRequest::EthSignTypedDataV4 {
        account: request.item.account,
        typed_data: json!({}),
        domain_chain_id: Some(U256::from(1_u64)),
    };

    let progress = WalletConnectApprovalProgress::new(4, &request);

    assert_eq!(
        progress
            .steps
            .iter()
            .map(|step| step.step)
            .collect::<Vec<_>>(),
        vec![
            WalletConnectApprovalProgressStep::ApproveOnDevice,
            WalletConnectApprovalProgressStep::RespondToDapp,
        ]
    );
    assert_eq!(progress.steps[0].status, PublicActionStepStatus::Pending);
    assert_eq!(progress.steps[1].status, PublicActionStepStatus::NotStarted);
}

#[test]
fn walletconnect_hash_fallback_warning_uses_explicit_continue_label() {
    let mut request = test_walletconnect_request("session-topic:7", Some(1_700_000_300));
    request.account_source = PublicAccountSource::HardwareDerived;
    request.item.method = WalletConnectSupportedMethod::EthSignTypedDataV4;

    assert!(
        !walletconnect_request_uses_hardware_typed_data_hash_fallback(
            &request,
            HardwareTypedDataSigningMode::ClearSign,
        )
    );
    assert_eq!(
        walletconnect_request_approve_label(false, true, false),
        "Approve on device"
    );
    assert!(
        walletconnect_request_uses_hardware_typed_data_hash_fallback(
            &request,
            HardwareTypedDataSigningMode::Eip712HashFallback,
        )
    );
    assert_eq!(
        walletconnect_request_approve_label(false, true, true),
        "Continue with hash fallback"
    );
    assert_eq!(
        walletconnect_request_approve_label(true, true, true),
        "Waiting for device..."
    );
}

#[test]
fn walletconnect_hash_fallback_mode_uses_request_session_account() {
    let (root_dir, store) = walletconnect_test_store();
    let wallet_id = "wc-hardware-fallback-mode";
    let profile_fingerprint = "ledger:evm:0x1111111111111111111111111111111111111111";
    let descriptor = HardwareDerivationDescriptor::ledger_eip1024_v1(
        parse_bip32_path("m/44'/60'/0'/0/0").expect("hardware path"),
        0,
        profile_fingerprint.to_owned(),
        HardwareWalletSyncIntent::CreateNew,
    );
    let metadata = store
        .new_hardware_wallet_metadata(TEST_PASSWORD, wallet_id, "Hardware wallet", descriptor)
        .expect("hardware metadata");
    let view_key = HardwareViewAccessKey::new([9u8; 32]);
    store
        .store_hardware_derived_wallet_from_entropy_with_metadata(
            TEST_PASSWORD,
            wallet_id,
            0,
            &[7u8; 32],
            &metadata,
            &view_key,
        )
        .expect("store hardware wallet");
    let mut hardware_session = store
        .hardware_profile_session_for_fingerprint(
            TEST_PASSWORD,
            HardwareDeviceKind::Ledger,
            profile_fingerprint,
            None,
        )
        .expect("hardware profile session");
    let public_descriptor =
        HardwarePublicAccountDescriptor::for_wallet_public_index(HardwareDeviceKind::Ledger, 0, 0)
            .expect("hardware public descriptor");
    hardware_session
        .cache_typed_data_signing_mode(
            &public_descriptor,
            HardwareTypedDataSigningMode::Eip712HashFallback,
        )
        .expect("cache fallback mode");
    let view_session = store
        .load_hardware_view_session(TEST_PASSWORD, &hardware_session, wallet_id, &view_key)
        .expect("hardware view session");
    let mut request = test_walletconnect_request("session-topic:7", Some(1_700_000_300));
    request.account_source = PublicAccountSource::HardwareDerived;
    request.item.method = WalletConnectSupportedMethod::EthSignTypedDataV4;
    request.session.selected_public_account_uuid = "hardware-account-a".to_owned();
    request.session.selected_public_account_scope = PublicAccountScope::Global;
    let other_account = PublicAccountMetadata {
        public_account_uuid: "selected-account-b".to_owned(),
        address: alloy::primitives::Address::from([0x22; 20]),
        label: None,
        source: PublicAccountSource::Imported,
        scope: PublicAccountScope::Global,
        derivation_index: None,
        hardware_descriptor: None,
        status: PublicAccountStatus::Active,
        display_order: 0,
    };
    let request_account = PublicAccountMetadata {
        public_account_uuid: "hardware-account-a".to_owned(),
        address: request.item.account,
        label: None,
        source: PublicAccountSource::HardwareDerived,
        scope: PublicAccountScope::Global,
        derivation_index: Some(0),
        hardware_descriptor: Some(public_descriptor),
        status: PublicAccountStatus::Active,
        display_order: 1,
    };
    let public_accounts = vec![other_account, request_account];

    assert_eq!(
        walletconnect_hardware_typed_data_mode_for_request(
            &request,
            &public_accounts,
            Some(&view_session),
        ),
        HardwareTypedDataSigningMode::Eip712HashFallback
    );

    drop(store);
    fs::remove_dir_all(root_dir).expect("remove temp db dir");
}

#[test]
fn walletconnect_personal_sign_progress_fails_response_after_device_approval() {
    let mut request = test_walletconnect_request("session-topic:7", Some(1_700_000_300));
    request.parsed = WalletConnectParsedRequest::PersonalSign {
        message: "0x68656c6c6f".to_owned(),
        account: request.item.account,
    };
    let mut progress = WalletConnectApprovalProgress::new(3, &request);

    progress.apply_update(
        WalletConnectApprovalProgressStep::ApproveOnDevice,
        PublicActionStepStatus::Done,
        None,
        None,
    );
    progress.apply_update(
        WalletConnectApprovalProgressStep::RespondToDapp,
        PublicActionStepStatus::Pending,
        None,
        None,
    );
    progress.fail("relay response timed out".to_owned());

    assert_eq!(progress.steps[0].status, PublicActionStepStatus::Done);
    assert_eq!(progress.steps[1].status, PublicActionStepStatus::Error);
    assert_eq!(
        progress.steps[1].message.as_deref(),
        Some("relay response timed out")
    );
}

#[test]
fn walletconnect_completed_transaction_result_keeps_copyable_tx_hash() {
    let request = test_walletconnect_request("session-topic:7", Some(1_700_000_300));
    let outcome = WalletConnectRequestApprovalOutcome {
        authorization_failed: false,
        response_published: true,
        submitted_tx_hash: Some("0xabcdef".to_owned()),
        relay_error: None,
        request_error: None,
        expired: false,
        hash_fallback_confirmation_required: false,
        refreshed_hardware_session: None,
    };

    let completed = WalletConnectCompletedRequestUi::from_outcome(request, &outcome);

    assert_eq!(
        completed.status,
        WalletConnectCompletedRequestStatus::TransactionSubmitted
    );
    assert_eq!(completed.submitted_tx_hash.as_deref(), Some("0xabcdef"));
    assert_eq!(
        completed.message.as_ref(),
        "Transaction submitted and WalletConnect response published."
    );
}

#[test]
fn walletconnect_completed_transaction_result_warns_when_response_publish_fails() {
    let request = test_walletconnect_request("session-topic:7", Some(1_700_000_300));
    let outcome = WalletConnectRequestApprovalOutcome {
        authorization_failed: false,
        response_published: false,
        submitted_tx_hash: Some("0xabcdef".to_owned()),
        relay_error: Some("relay response timed out".to_owned()),
        request_error: None,
        expired: false,
        hash_fallback_confirmation_required: false,
        refreshed_hardware_session: None,
    };

    let completed = WalletConnectCompletedRequestUi::from_outcome(request, &outcome);

    assert_eq!(
        completed.status,
        WalletConnectCompletedRequestStatus::TransactionSubmittedRelayResponseFailed
    );
    assert_eq!(completed.submitted_tx_hash.as_deref(), Some("0xabcdef"));
    assert_eq!(completed.error.as_deref(), Some("relay response timed out"));
}

#[test]
fn walletconnect_completed_request_error_is_not_shown_as_approved() {
    let request = test_walletconnect_request("session-topic:7", Some(1_700_000_300));
    let outcome = WalletConnectRequestApprovalOutcome {
        authorization_failed: false,
        response_published: true,
        submitted_tx_hash: None,
        relay_error: None,
        request_error: Some("Trezor ActionCancelled".to_owned()),
        expired: false,
        hash_fallback_confirmation_required: false,
        refreshed_hardware_session: None,
    };

    let completed = WalletConnectCompletedRequestUi::from_outcome(request, &outcome);

    assert_eq!(
        completed.status,
        WalletConnectCompletedRequestStatus::RequestFailed
    );
    assert_eq!(completed.error.as_deref(), Some("Trezor ActionCancelled"));
    assert_eq!(
        completed.message.as_ref(),
        "Request was not approved; error response published to the dapp."
    );
}

#[test]
fn walletconnect_hash_fallback_confirmation_outcome_is_local_retry() {
    let outcome = WalletConnectRequestApprovalOutcome::hash_fallback_confirmation_required(None);

    assert!(outcome.hash_fallback_confirmation_required);
    assert!(!outcome.response_published);
    assert!(outcome.submitted_tx_hash.is_none());
    assert!(outcome.relay_error.is_none());
    assert!(outcome.request_error.is_none());
}

#[test]
fn relay_worker_emits_interleaved_subscription_pushes() {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let mut output = WalletConnectRelayOutput::default();

    walletconnect_route_subscription_payloads(
        vec![WalletConnectRelaySubscriptionPayload {
            id: "sub-1".to_owned(),
            topic: "session-topic".to_owned(),
            message: "encrypted-push".to_owned(),
        }],
        &mut output,
        Some(&event_tx),
        true,
        None,
    );

    assert!(output.messages.is_empty());
    let event = event_rx.try_recv().expect("worker output event");
    let WalletConnectRelayWorkerEvent::Output(output) = event else {
        panic!("expected output event");
    };
    assert_eq!(output.messages.len(), 1);
    assert_eq!(output.messages[0].topic, "session-topic");
    assert_eq!(output.messages[0].message, "encrypted-push");
}

#[test]
fn relay_worker_splits_interleaved_pushes_by_command_topic() {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let mut output = WalletConnectRelayOutput::default();
    let command_topics = BTreeSet::from(["pairing-topic".to_owned()]);

    walletconnect_route_subscription_payloads(
        vec![
            WalletConnectRelaySubscriptionPayload {
                id: "sub-pairing".to_owned(),
                topic: "pairing-topic".to_owned(),
                message: "encrypted-proposal".to_owned(),
            },
            WalletConnectRelaySubscriptionPayload {
                id: "sub-session".to_owned(),
                topic: "session-topic".to_owned(),
                message: "encrypted-session-request".to_owned(),
            },
        ],
        &mut output,
        Some(&event_tx),
        false,
        Some(&command_topics),
    );

    assert_eq!(output.messages.len(), 1);
    assert_eq!(output.messages[0].topic, "pairing-topic");
    assert_eq!(output.messages[0].message, "encrypted-proposal");
    let event = event_rx.try_recv().expect("worker output event");
    let WalletConnectRelayWorkerEvent::Output(output) = event else {
        panic!("expected output event");
    };
    assert_eq!(output.messages.len(), 1);
    assert_eq!(output.messages[0].topic, "session-topic");
    assert_eq!(output.messages[0].message, "encrypted-session-request");
    assert!(event_rx.try_recv().is_err());
}

#[test]
fn relay_error_path_emits_buffered_subscription_pushes() {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let mut output = WalletConnectRelayOutput::default();

    walletconnect_route_subscription_payloads(
        vec![WalletConnectRelaySubscriptionPayload {
            id: "sub-1".to_owned(),
            topic: "session-topic".to_owned(),
            message: "encrypted-push-before-timeout".to_owned(),
        }],
        &mut output,
        Some(&event_tx),
        true,
        None,
    );

    assert!(output.messages.is_empty());
    let event = event_rx.try_recv().expect("worker output event");
    let WalletConnectRelayWorkerEvent::Output(output) = event else {
        panic!("expected output event");
    };
    assert_eq!(output.messages.len(), 1);
    assert_eq!(output.messages[0].topic, "session-topic");
    assert_eq!(output.messages[0].message, "encrypted-push-before-timeout");
}

#[test]
fn relay_command_error_emits_accumulated_command_topic_pushes_once() {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let command_topics = BTreeSet::from(["command-topic".to_owned()]);
    let mut output = WalletConnectRelayOutput::default();

    walletconnect_route_subscription_payloads(
        vec![WalletConnectRelaySubscriptionPayload {
            id: "sub-1".to_owned(),
            topic: "command-topic".to_owned(),
            message: "step-one-command-message".to_owned(),
        }],
        &mut output,
        Some(&event_tx),
        false,
        Some(&command_topics),
    );
    walletconnect_route_subscription_payloads(
        vec![WalletConnectRelaySubscriptionPayload {
            id: "sub-1".to_owned(),
            topic: "command-topic".to_owned(),
            message: "failing-step-command-message".to_owned(),
        }],
        &mut output,
        Some(&event_tx),
        false,
        Some(&command_topics),
    );

    walletconnect_emit_accumulated_output_messages(&mut output, Some(&event_tx));

    assert!(output.messages.is_empty());
    let event = event_rx.try_recv().expect("worker output event");
    let WalletConnectRelayWorkerEvent::Output(output) = event else {
        panic!("expected output event");
    };
    assert_eq!(output.messages.len(), 2);
    assert_eq!(output.messages[0].topic, "command-topic");
    assert_eq!(output.messages[0].message, "step-one-command-message");
    assert_eq!(output.messages[1].topic, "command-topic");
    assert_eq!(output.messages[1].message, "failing-step-command-message");
    assert!(event_rx.try_recv().is_err());
}

#[tokio::test]
async fn publish_response_routes_through_worker_command() {
    let (command_tx, mut command_rx) = mpsc::unbounded_channel();
    let worker = WalletConnectRelayWorkerHandle {
        worker_id: 1,
        project_id: "project-a".to_owned(),
        command_tx,
    };
    let publish_worker = worker.clone();
    let publish = tokio::spawn(async move {
        publish_walletconnect_session_response_ref(
            &publish_worker,
            "session-topic".to_owned(),
            &[7u8; 32],
            WalletConnectJsonRpcResponse::success(44, json!(true)),
            WALLETCONNECT_RELAY_TTL_SECS,
            WC_SESSION_REQUEST_RESPONSE_TAG,
        )
        .await
    });

    let command = command_rx.recv().await.expect("worker command");
    let WalletConnectRelayWorkerCommand::Execute {
        steps,
        wait_for_push,
        emit_pushes,
        response_tx,
    } = command
    else {
        panic!("expected execute command");
    };
    assert!(!wait_for_push);
    assert!(emit_pushes);
    assert!(matches!(
        &steps[0],
        WalletConnectRelayStep::Publish(WalletConnectRelayRpc::Publish { topic, ttl, tag, .. })
            if topic == "session-topic"
                && *ttl == WALLETCONNECT_RELAY_TTL_SECS
                && *tag == WC_SESSION_REQUEST_RESPONSE_TAG
    ));
    let _ = response_tx.send(Ok(WalletConnectRelayOutput::default()));

    assert!(publish.await.expect("publish task").is_ok());
}

#[test]
fn relay_worker_stop_command_does_not_unsubscribe() {
    let (command_tx, mut command_rx) = mpsc::unbounded_channel();
    let worker = WalletConnectRelayWorkerHandle {
        worker_id: 1,
        project_id: "project-a".to_owned(),
        command_tx,
    };

    worker.stop();

    assert!(matches!(
        command_rx.try_recv().expect("stop command"),
        WalletConnectRelayWorkerCommand::Stop
    ));
}

#[test]
fn walletconnect_authorization_summary_includes_request_context() {
    let account = alloy::primitives::Address::from([0x11; 20]);
    let request = WalletConnectRequestUi {
        key: "session-topic:7".to_owned(),
        review_token: 1,
        session: test_walletconnect_session("session-topic"),
        parsed: WalletConnectParsedRequest::EthAccounts,
        item: WalletConnectPendingRequest {
            id: 7,
            topic: "session-topic".to_owned(),
            dapp_name: "Aave".to_owned(),
            chain_id: "eip155:1".to_owned(),
            method: WalletConnectSupportedMethod::EthSendTransaction,
            account,
            decoded_summary: Some(WalletConnectErc20CallSummary::Approve {
                spender: alloy::primitives::Address::from([0x22; 20]),
                amount: U256::from(1),
            }),
            raw_details: json!({ "to": "0xdAC17F958D2ee523a2206206994597C13D831ec7" }),
            expiry_timestamp: Some(1_700_000_300),
        },
        account_source: PublicAccountSource::Imported,
    };

    let summary = walletconnect_request_authorization_summary(&request);
    let rows = summary.rows_for_test();

    assert_eq!(summary.title_for_test(), "Authorize WalletConnect request");
    assert!(summary.detail_for_test().contains("eth_sendTransaction"));
    assert!(rows.contains(&("Dapp".to_owned(), "Aave".to_owned())));
    assert!(rows.contains(&("Method".to_owned(), "eth_sendTransaction".to_owned())));
    assert!(
        rows.iter().any(|(label, value)| {
            label == "Decoded request" && value.contains("ERC-20 approve")
        })
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
    let delete_message = test_walletconnect_relay_message(&session, delete);
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
    let ping_message = test_walletconnect_relay_message(&session, ping);
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

#[test]
fn switch_chain_event_uses_eip1193_hex_chain_id() {
    let (root_dir, store) = walletconnect_test_store();
    let view_session = import_test_wallet(&store, "wc-switch-event", "WC Switch Event");
    let account = store
        .import_public_account(
            TEST_PASSWORD,
            &view_session,
            TEST_IMPORTED_PRIVATE_KEY,
            Some("Switch WC"),
            true,
        )
        .expect("import public account");
    let mut session = test_walletconnect_session("switch-session-topic");
    session.selected_public_account_uuid = account.public_account_uuid.clone();
    session.selected_public_account_scope = account.scope.clone();
    session.approved_namespaces.insert(
        "eip155".to_owned(),
        WalletConnectApprovedNamespace {
            chains: vec!["eip155:1".to_owned(), "eip155:42161".to_owned()],
            accounts: vec![
                format!("eip155:1:{}", account.address),
                format!("eip155:42161:{}", account.address),
            ],
            methods: vec!["wallet_switchEthereumChain".to_owned()],
            events: vec!["chainChanged".to_owned()],
        },
    );
    let request = WalletConnectJsonRpcRequest::new(
        93,
        "wc_sessionRequest",
        json!({
            "chainId": "eip155:1",
            "request": {
                "method": "wallet_switchEthereumChain",
                "params": [{ "chainId": "0xa4b1" }]
            }
        }),
    );
    let relay_message = test_walletconnect_relay_message(&session, request);

    let outcome = process_walletconnect_session_message(
        &store,
        &view_session,
        &session,
        &relay_message,
        &BTreeSet::from([1, 42161]),
        current_unix_seconds(),
    )
    .expect("process switch chain");

    let SessionMessageOutcome::Respond {
        response,
        post_response_requests,
        ..
    } = outcome
    else {
        panic!("expected switch response");
    };
    assert_eq!(response.error, None);
    assert_eq!(post_response_requests.len(), 1);
    assert_eq!(
        post_response_requests[0].params["event"]["data"],
        json!("0xa4b1")
    );

    drop(store);
    fs::remove_dir_all(root_dir).expect("remove temp db dir");
}

#[test]
fn malformed_send_transaction_params_return_invalid_params_error() {
    let (root_dir, store) = walletconnect_test_store();
    let view_session = import_test_wallet(&store, "wc-malformed-params", "WC Malformed Params");
    let session = test_walletconnect_session("malformed-session-topic");
    let request = WalletConnectJsonRpcRequest::new(
        94,
        "wc_sessionRequest",
        json!({
            "chainId": "eip155:1",
            "request": {
                "method": "eth_sendTransaction",
                "params": [{
                    "from": "0x1111111111111111111111111111111111111111",
                    "data": "0xzz"
                }]
            }
        }),
    );
    let relay_message = test_walletconnect_relay_message(&session, request);

    let outcome = process_walletconnect_session_message(
        &store,
        &view_session,
        &session,
        &relay_message,
        &BTreeSet::from([1]),
        current_unix_seconds(),
    )
    .expect("process malformed request");

    let SessionMessageOutcome::Respond { response, .. } = outcome else {
        panic!("expected error response");
    };
    let error = response.error.expect("json-rpc error");
    assert_eq!(
        error.code,
        WalletConnectRequestErrorKind::MalformedParams.code()
    );
    assert!(error.message.contains("transaction data must be valid hex"));

    drop(store);
    fs::remove_dir_all(root_dir).expect("remove temp db dir");
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

#[test]
fn walletconnect_transaction_request_preserves_explicit_execution_fields() {
    let from = alloy::primitives::Address::from([0x11; 20]);
    let to = alloy::primitives::Address::from([0x22; 20]);
    let access_list: alloy::rpc::types::transaction::AccessList = serde_json::from_value(json!([
        {
            "address": to.to_string(),
            "storageKeys": ["0x0000000000000000000000000000000000000000000000000000000000000003"]
        }
    ]))
    .unwrap();
    let tx = transaction_request_from_walletconnect(
        1,
        WalletConnectEvmTransaction {
            from,
            to: Some(to),
            value: Some(U256::from(5_u64)),
            data: None,
            access_list: Some(access_list.clone()),
            gas: Some(U256::from(21_000_u64)),
            gas_price: None,
            max_fee_per_gas: Some(U256::from(20_000_000_000_u64)),
            max_priority_fee_per_gas: Some(U256::from(2_000_000_000_u64)),
            chain_id: Some(1),
            nonce: Some(U256::from(7_u64)),
            transaction_type: Some(1),
            raw: json!({}),
        },
    )
    .unwrap();

    assert_eq!(tx.chain_id, Some(1));
    assert_eq!(tx.from, Some(from));
    assert_eq!(tx.to, Some(to.into()));
    assert_eq!(tx.value, Some(U256::from(5_u64)));
    assert_eq!(tx.gas, Some(21_000));
    assert_eq!(tx.max_fee_per_gas, Some(20_000_000_000));
    assert_eq!(tx.max_priority_fee_per_gas, Some(2_000_000_000));
    assert_eq!(tx.nonce, Some(7));
    assert_eq!(tx.access_list, Some(access_list));
    assert_eq!(tx.transaction_type, Some(1));
}

#[test]
fn personal_sign_message_bytes_decode_only_explicit_hex_prefix() {
    assert_eq!(walletconnect_personal_message_bytes("0x616263"), b"abc");
    assert_eq!(walletconnect_personal_message_bytes("616263"), b"616263");
    assert_eq!(
        walletconnect_personal_message_bytes("deadbeef"),
        b"deadbeef"
    );
}

#[test]
fn submitted_transaction_expiry_boundary_keeps_result_response() {
    assert!(!walletconnect_approval_should_publish_expired_response(
        Some(1_700_000_000),
        1_700_000_000,
        Some("0xabc")
    ));
    assert!(walletconnect_approval_should_publish_expired_response(
        Some(1_700_000_000),
        1_700_000_000,
        None
    ));
    assert!(!walletconnect_approval_should_publish_expired_response(
        Some(1_700_000_001),
        1_700_000_000,
        None
    ));
}

#[test]
fn send_transaction_approval_does_not_use_expiry_timeout_after_authorization() {
    let mut request = test_walletconnect_request("session-topic:7", Some(1_700_000_300));
    request.parsed = WalletConnectParsedRequest::EthSendTransaction {
        transaction: WalletConnectEvmTransaction {
            from: request.item.account,
            to: None,
            value: None,
            data: None,
            access_list: None,
            gas: None,
            gas_price: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            chain_id: None,
            nonce: None,
            transaction_type: None,
            raw: json!({}),
        },
    };
    let personal_sign = WalletConnectParsedRequest::PersonalSign {
        message: "0x68656c6c6f".to_owned(),
        account: request.item.account,
    };

    assert!(!walletconnect_request_approval_uses_expiry_timeout(
        &request.parsed
    ));
    assert!(walletconnect_request_approval_uses_expiry_timeout(
        &personal_sign
    ));
}

#[test]
fn expired_request_keys_skip_unexpired_and_in_flight_requests() {
    let mut pending_requests = BTreeMap::new();
    pending_requests.insert(
        "session-topic:1".to_owned(),
        test_walletconnect_request("session-topic:1", Some(1_700_000_010)),
    );
    pending_requests.insert(
        "session-topic:2".to_owned(),
        test_walletconnect_request("session-topic:2", Some(1_700_000_020)),
    );
    pending_requests.insert(
        "session-topic:3".to_owned(),
        test_walletconnect_request("session-topic:3", None),
    );
    let mut request_actions = BTreeSet::new();
    request_actions.insert("session-topic:1".to_owned());

    let expired =
        expired_walletconnect_request_keys(&pending_requests, &request_actions, 1_700_000_011);

    assert!(expired.is_empty());

    let expired =
        expired_walletconnect_request_keys(&pending_requests, &request_actions, 1_700_000_021);

    assert_eq!(expired, vec!["session-topic:2".to_owned()]);
}

#[test]
fn handled_request_keys_suppress_completed_request_replay() {
    let mut pending_requests = BTreeMap::new();
    let mut handled_request_keys = BTreeSet::new();
    let mut handled_request_key_order = VecDeque::new();
    let request = test_walletconnect_request("session-topic:7", None);

    assert!(walletconnect_request_should_queue(
        &pending_requests,
        &handled_request_keys,
        &request.key
    ));

    pending_requests.insert(request.key.clone(), request.clone());
    assert!(!walletconnect_request_should_queue(
        &pending_requests,
        &handled_request_keys,
        &request.key
    ));

    pending_requests.clear();
    remember_walletconnect_handled_request_key(
        &mut handled_request_keys,
        &mut handled_request_key_order,
        request.key.clone(),
        8,
    );

    assert!(!walletconnect_request_should_queue(
        &pending_requests,
        &handled_request_keys,
        &request.key
    ));
}

#[test]
fn auto_open_request_key_skips_dismissed_requests() {
    let mut pending_requests = BTreeMap::new();
    pending_requests.insert(
        "session-topic:1".to_owned(),
        test_walletconnect_request("session-topic:1", None),
    );
    pending_requests.insert(
        "session-topic:2".to_owned(),
        test_walletconnect_request("session-topic:2", None),
    );
    let mut dismissed = BTreeSet::new();

    assert_eq!(
        next_walletconnect_auto_open_request_key(&pending_requests, &dismissed).as_deref(),
        Some("session-topic:1")
    );

    dismissed.insert("session-topic:1".to_owned());
    assert_eq!(
        next_walletconnect_auto_open_request_key(&pending_requests, &dismissed).as_deref(),
        Some("session-topic:2")
    );

    dismissed.insert("session-topic:2".to_owned());
    assert_eq!(
        next_walletconnect_auto_open_request_key(&pending_requests, &dismissed),
        None
    );
}

#[test]
fn manual_request_key_selection_includes_dismissed_requests() {
    let mut pending_requests = BTreeMap::new();
    pending_requests.insert(
        "session-topic:1".to_owned(),
        test_walletconnect_request("session-topic:1", None),
    );

    assert_eq!(
        first_walletconnect_pending_request_key(&pending_requests).as_deref(),
        Some("session-topic:1")
    );
}

#[test]
fn request_dialog_nav_reports_position_and_edges() {
    let mut pending_requests = BTreeMap::new();
    pending_requests.insert(
        "session-topic:1".to_owned(),
        test_walletconnect_request("session-topic:1", None),
    );
    pending_requests.insert(
        "session-topic:2".to_owned(),
        test_walletconnect_request("session-topic:2", None),
    );
    pending_requests.insert(
        "session-topic:3".to_owned(),
        test_walletconnect_request("session-topic:3", None),
    );

    assert_eq!(
        walletconnect_request_dialog_nav(&pending_requests, "session-topic:1"),
        Some(WalletConnectRequestDialogNav {
            index: 1,
            total: 3,
            previous_key: None,
            next_key: Some("session-topic:2".to_owned()),
        })
    );
    assert_eq!(
        walletconnect_request_dialog_nav(&pending_requests, "session-topic:2"),
        Some(WalletConnectRequestDialogNav {
            index: 2,
            total: 3,
            previous_key: Some("session-topic:1".to_owned()),
            next_key: Some("session-topic:3".to_owned()),
        })
    );
    assert_eq!(
        walletconnect_request_dialog_nav(&pending_requests, "session-topic:3"),
        Some(WalletConnectRequestDialogNav {
            index: 3,
            total: 3,
            previous_key: Some("session-topic:2".to_owned()),
            next_key: None,
        })
    );
}

#[test]
fn request_dialog_nav_returns_none_for_missing_request() {
    let mut pending_requests = BTreeMap::new();
    pending_requests.insert(
        "session-topic:1".to_owned(),
        test_walletconnect_request("session-topic:1", None),
    );

    assert_eq!(
        walletconnect_request_dialog_nav(&pending_requests, "session-topic:missing"),
        None
    );
}

#[test]
fn pending_request_expiry_allows_less_than_protocol_minimum_remaining() {
    assert!(
        walletconnect_validate_pending_request_expiry(Some(1_700_000_240), 1_700_000_000).is_ok()
    );

    let error = walletconnect_validate_pending_request_expiry(Some(1_700_000_000), 1_700_000_000)
        .expect_err("expired request");
    assert_eq!(error.kind, WalletConnectRequestErrorKind::ExpiredRequest);
}

#[tokio::test]
async fn request_expiry_deadline_cancels_slow_approval_future() {
    let result =
        walletconnect_await_before_request_expiry(Some(current_unix_seconds() + 1), async {
            tokio::time::sleep(Duration::from_secs(2)).await;
            "completed"
        })
        .await;

    assert_eq!(result, Err(WalletConnectRequestExpired));
}

#[tokio::test]
async fn expired_approval_task_publishes_expired_response() {
    let (root_dir, store) = walletconnect_test_store();
    let store = Arc::new(store);
    let view_session = Arc::new(import_test_wallet(
        store.as_ref(),
        "wc-expired-approval",
        "WC Expired Approval",
    ));
    let mut request =
        test_walletconnect_request("session-topic:expired", Some(current_unix_seconds()));
    request.session.selected_public_account_uuid = "public-account".to_owned();
    let request_id = request.item.id;
    let session_topic = request.session.session_topic.clone();
    let sym_key = request.session.keys.sym_key;
    let (command_tx, mut command_rx) = mpsc::unbounded_channel();
    let context = WalletConnectClientContext {
        worker: WalletConnectRelayWorkerHandle {
            worker_id: 1,
            project_id: "project-a".to_owned(),
            command_tx,
        },
    };
    let http = wallet_ops::build_http_client(None).expect("direct http context");

    let task_store = Arc::clone(&store);
    let task_view_session = Arc::clone(&view_session);
    let approval = tokio::spawn(async move {
        approve_walletconnect_request_task(
            request,
            task_store,
            task_view_session,
            Zeroizing::new(TEST_PASSWORD.to_owned()),
            None,
            None,
            None,
            context,
            http,
            false,
            None,
        )
        .await
    });

    let command = command_rx.recv().await.expect("expired response command");
    let WalletConnectRelayWorkerCommand::Execute {
        steps,
        wait_for_push,
        emit_pushes,
        response_tx,
    } = command
    else {
        panic!("expected execute command");
    };
    assert!(!wait_for_push);
    assert!(emit_pushes);
    let WalletConnectRelayStep::Publish(WalletConnectRelayRpc::Publish {
        topic,
        message,
        ttl,
        tag,
    }) = &steps[0]
    else {
        panic!("expected expired response publish");
    };
    assert_eq!(topic, &session_topic);
    assert_eq!(*ttl, WALLETCONNECT_RELAY_TTL_SECS);
    assert_eq!(*tag, WC_SESSION_REQUEST_RESPONSE_TAG);
    let envelope = wallet_ops::WalletConnectEnvelope::from_base64(message).expect("envelope");
    let plaintext = decode_walletconnect_message(&sym_key, &envelope).expect("expired response");
    let response: WalletConnectJsonRpcResponse<Value> =
        serde_json::from_slice(&plaintext).expect("response json");
    assert_eq!(response.id, request_id);
    assert_eq!(
        response.error.expect("expired error").code,
        WalletConnectRequestErrorKind::ExpiredRequest.code()
    );
    assert!(
        response_tx
            .send(Ok(WalletConnectRelayOutput::default()))
            .is_ok()
    );

    let outcome = approval
        .await
        .expect("approval task")
        .expect("approval task");

    assert!(outcome.expired);
    assert!(outcome.response_published);
    assert!(outcome.submitted_tx_hash.is_none());
    assert!(command_rx.try_recv().is_err());

    drop(view_session);
    drop(store);
    fs::remove_dir_all(root_dir).expect("remove temp db dir");
}

#[test]
fn expired_pairing_topics_include_silent_expired_pairings() {
    let expired_topic = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let active_topic = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let expired = WalletConnectPairingUri::parse_with_now(
            &format!(
                "wc:{expired_topic}@2?relay-protocol=irn&symKey=000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f&expiryTimestamp=10"
            ),
            1,
        )
        .expect("expired pairing");
    let active = WalletConnectPairingUri::parse_with_now(
            &format!(
                "wc:{active_topic}@2?relay-protocol=irn&symKey=000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f&expiryTimestamp=20"
            ),
            1,
        )
        .expect("active pairing");
    let pairings = BTreeMap::from([
        (expired.topic.clone(), expired),
        (active.topic.clone(), active),
    ]);

    assert_eq!(
        expired_walletconnect_pairing_topics(&pairings, 11),
        vec![expired_topic.to_owned()]
    );
}

#[test]
fn approved_chain_display_items_use_known_chain_names_and_icons() {
    let mut session = test_walletconnect_session("chain-display-topic");
    session
        .approved_namespaces
        .get_mut("eip155")
        .expect("eip155 namespace")
        .chains = vec![
        "eip155:42161".to_owned(),
        "eip155:1".to_owned(),
        "eip155:42161".to_owned(),
        "eip155:56".to_owned(),
    ];

    let items = approved_chain_display_items(&session);
    let labels = items
        .iter()
        .map(|item| item.label.as_str())
        .collect::<Vec<_>>();

    assert_eq!(labels, vec!["Arbitrum", "BSC", "Ethereum"]);
    assert!(items.iter().all(|item| item.icon_path.is_some()));
    assert!(labels.iter().all(|label| !label.contains("eip155")));
}

#[test]
fn approved_chain_display_items_fall_back_to_raw_unknown_chain() {
    let mut session = test_walletconnect_session("unknown-chain-display-topic");
    session
        .approved_namespaces
        .get_mut("eip155")
        .expect("eip155 namespace")
        .chains = vec!["eip155:10".to_owned()];

    let items = approved_chain_display_items(&session);

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].label, "eip155:10");
    assert_eq!(items[0].icon_path, None);
}

#[test]
fn format_unix_seconds_uses_local_datetime() {
    let formatted = format_unix_seconds(1_700_000_000);

    assert_ne!(formatted, "1700000000");
    assert!(formatted.contains('-'));
    assert!(formatted.contains(':'));
}

fn test_walletconnect_session(topic: &str) -> WalletConnectSessionRecord {
    let mut approved_namespaces = BTreeMap::new();
    approved_namespaces.insert(
        "eip155".to_owned(),
        WalletConnectApprovedNamespace {
            chains: vec!["eip155:1".to_owned()],
            accounts: vec!["eip155:1:0x1111111111111111111111111111111111111111".to_owned()],
            methods: vec!["eth_sendTransaction".to_owned()],
            events: vec!["accountsChanged".to_owned()],
        },
    );
    WalletConnectSessionRecord {
        session_uuid: "session-uuid".to_owned(),
        pairing_topic: "pairing-topic".to_owned(),
        session_topic: topic.to_owned(),
        relay_protocol: "irn".to_owned(),
        relay_client_id: "relay-client".to_owned(),
        peer_metadata: WalletConnectPeerMetadata {
            name: "Aave".to_owned(),
            description: String::new(),
            url: "https://app.aave.com".to_owned(),
            icons: Vec::new(),
        },
        approved_namespaces,
        selected_public_account_uuid: "public-account".to_owned(),
        selected_public_account_scope: PublicAccountScope::Global,
        owning_private_wallet_uuid: None,
        keys: WalletConnectSessionKeys {
            sym_key: [1u8; 32],
            responder_private_key: [2u8; 32],
            responder_public_key: [3u8; 32],
        },
        expiry_timestamp: current_unix_seconds() + 300,
        lifecycle_state: WalletConnectSessionLifecycleState::Active,
    }
}

fn test_walletconnect_relay_message(
    session: &WalletConnectSessionRecord,
    request: WalletConnectJsonRpcRequest<Value>,
) -> WalletConnectRelayMessage {
    let plaintext = serde_json::to_vec(&request).expect("request json");
    let message = encode_walletconnect_message(&session.keys.sym_key, &plaintext)
        .expect("encode request")
        .to_base64();
    WalletConnectRelayMessage {
        topic: session.session_topic.clone(),
        message,
    }
}

fn test_walletconnect_request(key: &str, expiry_timestamp: Option<u64>) -> WalletConnectRequestUi {
    let account = alloy::primitives::Address::from([0x11; 20]);
    WalletConnectRequestUi {
        key: key.to_owned(),
        review_token: 1,
        session: test_walletconnect_session("session-topic"),
        parsed: WalletConnectParsedRequest::EthAccounts,
        item: WalletConnectPendingRequest {
            id: 7,
            topic: "session-topic".to_owned(),
            dapp_name: "Aave".to_owned(),
            chain_id: "eip155:1".to_owned(),
            method: WalletConnectSupportedMethod::EthSendTransaction,
            account,
            decoded_summary: None,
            raw_details: json!({}),
            expiry_timestamp,
        },
        account_source: PublicAccountSource::Imported,
    }
}
