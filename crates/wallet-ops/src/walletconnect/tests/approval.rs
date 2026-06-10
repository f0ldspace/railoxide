use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::vault::{PublicAccountScope, WalletConnectRelayIdentity};
use crate::walletconnect::{
    WC_SESSION_SETTLE, WalletConnectApprovalMessages, WalletConnectError,
    WalletConnectProposalRejectionReason, WalletConnectRelayStep, approve_walletconnect_session,
    reject_walletconnect_session_proposal,
};

use super::helpers::{
    NOW, decode_encrypted_json, namespace, supported_chains, test_proposal, test_public_account,
};

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
