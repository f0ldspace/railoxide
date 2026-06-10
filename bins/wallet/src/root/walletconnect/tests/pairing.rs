use super::fixtures::*;
use super::*;

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
