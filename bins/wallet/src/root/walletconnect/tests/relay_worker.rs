use super::fixtures::*;
use super::*;

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
