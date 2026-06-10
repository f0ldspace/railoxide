use super::fixtures::*;
use super::*;

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
