use std::collections::BTreeMap;

use alloy::primitives::{U256, address};
use serde_json::json;

use crate::hardware::HardwareTypedDataSigningMode;
use crate::vault::{
    PublicAccountScope, PublicAccountSource, WalletConnectRelayIdentity,
    WalletConnectSessionAccountResolution,
};
use crate::walletconnect::{
    WalletConnectErc20CallSummary, WalletConnectError, WalletConnectNamespaceAccountSupport,
    WalletConnectParsedRequest, WalletConnectPendingRequestQueue, approve_walletconnect_session,
    approve_walletconnect_session_with_account_support, parse_walletconnect_session_request,
    validate_walletconnect_session_request,
    validate_walletconnect_session_request_with_account_support,
};

use super::helpers::{
    NOW, approved_request_session, namespace, supported_chains, test_proposal, test_public_account,
    typed_data_payload,
};

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
fn hardware_typed_data_request_validation_allows_unknown_capability_probe() {
    let mut required = BTreeMap::new();
    required.insert(
        "eip155".to_owned(),
        namespace(&["eip155:1"], &["eth_signTypedData_v4"], &[]),
    );
    let proposal = test_proposal(required);
    let relay_identity = WalletConnectRelayIdentity {
        signing_key: [8u8; 32],
        client_id: "relay-client".to_owned(),
    };
    let mut account = test_public_account(PublicAccountScope::Global);
    account.source = PublicAccountSource::HardwareDerived;
    let supported =
        WalletConnectNamespaceAccountSupport::hardware(HardwareTypedDataSigningMode::ClearSign);
    let approval = approve_walletconnect_session_with_account_support(
        &proposal,
        &[1u8; 32],
        &relay_identity,
        &account,
        supported,
        &supported_chains(&[1]),
        "hardware-typed-data-session",
        NOW,
    )
    .expect("approve typed-data session");
    let resolution = WalletConnectSessionAccountResolution::Usable(account.clone());
    let request = parse_walletconnect_session_request(
        33,
        "eth_signTypedData_v4",
        &json!([account.address.to_string(), typed_data_payload(&json!(1))]),
    )
    .expect("typed-data request");

    let validation = validate_walletconnect_session_request_with_account_support(
        &approval.session,
        &resolution,
        supported,
        &approval.session.session_topic,
        33,
        "eip155:1",
        request.clone(),
        Some(NOW + 300),
        NOW,
    )
    .expect("supported typed-data validation");
    assert_eq!(validation.request.method().as_str(), "eth_signTypedData_v4");

    let unknown_validation = validate_walletconnect_session_request_with_account_support(
        &approval.session,
        &resolution,
        WalletConnectNamespaceAccountSupport::hardware_typed_data_capability_unknown(),
        &approval.session.session_topic,
        33,
        "eip155:1",
        request.clone(),
        Some(NOW + 300),
        NOW,
    )
    .expect("unknown hardware typed-data capability can be probed at approval time");
    assert_eq!(
        unknown_validation.request.method().as_str(),
        "eth_signTypedData_v4"
    );
    assert!(unknown_validation.approval_item.is_some());

    assert!(matches!(
        validate_walletconnect_session_request_with_account_support(
            &approval.session,
            &resolution,
            WalletConnectNamespaceAccountSupport::hardware(HardwareTypedDataSigningMode::Unsupported),
            &approval.session.session_topic,
            33,
            "eip155:1",
            request,
            Some(NOW + 300),
            NOW,
        ),
        Err(WalletConnectError::UnsupportedMethod(method)) if method == "eth_signTypedData_v4"
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
            typed_data_payload(&json!("0xa"))
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
            typed_data_payload(&json!(
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
                typed_data_payload(&json!("0x10000000000000000000000000000000000000000000000000000000000000000"))
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
