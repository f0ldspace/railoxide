use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use alloy::hex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::vault::{
    PublicAccountMetadata, PublicAccountScope, WalletConnectPeerMetadata,
    WalletConnectRelayIdentity, WalletConnectSessionKeys, WalletConnectSessionLifecycleState,
    WalletConnectSessionRecord,
};

use super::crypto::{
    WalletConnectEnvelope, decode_walletconnect_message, derive_walletconnect_session_sym_key,
    derive_walletconnect_session_topic, generate_walletconnect_key_pair,
};
use super::namespace::{
    WalletConnectNamespaceAccountSupport, WalletConnectNamespaceNegotiation,
    WalletConnectNamespaceProposal, negotiate_walletconnect_namespaces_with_account_support,
};
use super::relay::{
    WalletConnectJsonRpcError, WalletConnectJsonRpcRequest, WalletConnectJsonRpcResponse,
};
use super::session::WalletConnectApprovalMessages;
use super::uri::{WALLETCONNECT_IRN_RELAY_PROTOCOL, WalletConnectPairingUri};
use super::{Result, WalletConnectError, WalletConnectRelayStep};

const WALLETCONNECT_SESSION_EXPIRY_SECS: u64 = 604_800;
const WALLETCONNECT_PROPOSAL_REVIEW_EXPIRY_SECS: u64 = 300;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletConnectPairingStart {
    pub uri: WalletConnectPairingUri,
    pub relay_steps: Vec<WalletConnectRelayStep>,
}

pub fn start_walletconnect_pairing(
    input: &str,
    now_unix_seconds: u64,
) -> Result<WalletConnectPairingStart> {
    let uri = WalletConnectPairingUri::parse_with_now(input, now_unix_seconds)?;
    let relay_steps = vec![
        WalletConnectRelayStep::FetchMessages {
            topic: uri.topic.clone(),
        },
        WalletConnectRelayStep::Subscribe {
            topic: uri.topic.clone(),
        },
    ];
    Ok(WalletConnectPairingStart { uri, relay_steps })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalletConnectSessionProposal {
    pub id: u64,
    pub pairing_topic: String,
    pub proposer_public_key: String,
    pub relay_protocol: String,
    pub peer_metadata: WalletConnectPeerMetadata,
    pub required_namespaces: BTreeMap<String, WalletConnectNamespaceProposal>,
    pub optional_namespaces: BTreeMap<String, WalletConnectNamespaceProposal>,
    pub expiry_timestamp: u64,
}

impl WalletConnectSessionProposal {
    #[must_use]
    pub const fn is_expired(&self, now_unix_seconds: u64) -> bool {
        self.expiry_timestamp <= now_unix_seconds
    }

    #[must_use]
    pub fn summary(&self, now_unix_seconds: u64) -> WalletConnectProposalSummary {
        WalletConnectProposalSummary {
            dapp_name: self.peer_metadata.name.clone(),
            dapp_url: self.peer_metadata.url.clone(),
            required_namespace_keys: self.required_namespaces.keys().cloned().collect(),
            optional_namespace_keys: self.optional_namespaces.keys().cloned().collect(),
            expired: self.is_expired(now_unix_seconds),
            expiry_timestamp: self.expiry_timestamp,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletConnectProposalSummary {
    pub dapp_name: String,
    pub dapp_url: String,
    pub required_namespace_keys: Vec<String>,
    pub optional_namespace_keys: Vec<String>,
    pub expired: bool,
    pub expiry_timestamp: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionProposeParams {
    required_namespaces: BTreeMap<String, WalletConnectNamespaceProposal>,
    #[serde(default)]
    optional_namespaces: BTreeMap<String, WalletConnectNamespaceProposal>,
    proposer: SessionProposer,
    #[serde(default)]
    relays: Vec<SessionRelay>,
    expiry_timestamp: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionProposer {
    public_key: String,
    metadata: WalletConnectPeerMetadata,
}

#[derive(Debug, Deserialize)]
struct SessionRelay {
    protocol: String,
}

pub fn decode_walletconnect_session_proposal(
    pairing: &WalletConnectPairingUri,
    encoded_message: &str,
) -> Result<WalletConnectSessionProposal> {
    let envelope = WalletConnectEnvelope::from_base64(encoded_message)?;
    let plaintext = decode_walletconnect_message(&pairing.sym_key, &envelope)?;
    let request: WalletConnectJsonRpcRequest<SessionProposeParams> =
        serde_json::from_slice(&plaintext)?;
    if request.method != "wc_sessionPropose" {
        return Err(WalletConnectError::UnsupportedMethod(request.method));
    }
    let relay_protocol = request.params.relays.first().map_or_else(
        || WALLETCONNECT_IRN_RELAY_PROTOCOL.to_owned(),
        |relay| relay.protocol.clone(),
    );
    if relay_protocol != WALLETCONNECT_IRN_RELAY_PROTOCOL {
        return Err(WalletConnectError::InvalidUri(
            "session proposal relay protocol must be irn".to_owned(),
        ));
    }
    Ok(WalletConnectSessionProposal {
        id: request.id,
        pairing_topic: pairing.topic.clone(),
        proposer_public_key: request.params.proposer.public_key,
        relay_protocol,
        peer_metadata: request.params.proposer.metadata,
        required_namespaces: request.params.required_namespaces,
        optional_namespaces: request.params.optional_namespaces,
        expiry_timestamp: request.params.expiry_timestamp.unwrap_or_else(|| {
            current_unix_seconds().saturating_add(WALLETCONNECT_PROPOSAL_REVIEW_EXPIRY_SECS)
        }),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletConnectSessionApproval {
    pub negotiation: WalletConnectNamespaceNegotiation,
    pub session: WalletConnectSessionRecord,
    pub approval_messages: WalletConnectApprovalMessages,
    pub relay_steps: Vec<WalletConnectRelayStep>,
}

pub fn approve_walletconnect_session(
    proposal: &WalletConnectSessionProposal,
    pairing_sym_key: &[u8; 32],
    relay_identity: &WalletConnectRelayIdentity,
    selected_account: &PublicAccountMetadata,
    supported_chain_ids: &BTreeSet<u64>,
    session_uuid: impl Into<String>,
    now_unix_seconds: u64,
) -> Result<WalletConnectSessionApproval> {
    approve_walletconnect_session_with_account_support(
        proposal,
        pairing_sym_key,
        relay_identity,
        selected_account,
        WalletConnectNamespaceAccountSupport::for_account_source(selected_account.source),
        supported_chain_ids,
        session_uuid,
        now_unix_seconds,
    )
}

pub fn approve_walletconnect_session_with_account_support(
    proposal: &WalletConnectSessionProposal,
    pairing_sym_key: &[u8; 32],
    relay_identity: &WalletConnectRelayIdentity,
    selected_account: &PublicAccountMetadata,
    selected_account_support: WalletConnectNamespaceAccountSupport,
    supported_chain_ids: &BTreeSet<u64>,
    session_uuid: impl Into<String>,
    now_unix_seconds: u64,
) -> Result<WalletConnectSessionApproval> {
    if proposal.is_expired(now_unix_seconds) {
        return Err(WalletConnectError::ExpiredUri);
    }
    if selected_account.status != crate::vault::PublicAccountStatus::Active {
        return Err(WalletConnectError::Relay(
            "selected Public account is not active".to_owned(),
        ));
    }

    let negotiation = negotiate_walletconnect_namespaces_with_account_support(
        &proposal.required_namespaces,
        &proposal.optional_namespaces,
        supported_chain_ids,
        selected_account.address,
        selected_account_support,
    )?;

    let proposer_public_key = parse_hex_32(&proposal.proposer_public_key)?;
    let (responder_private_key, responder_public_key) = generate_walletconnect_key_pair()?;
    let sym_key =
        derive_walletconnect_session_sym_key(&responder_private_key, &proposer_public_key)?;
    let session_topic = derive_walletconnect_session_topic(&sym_key);
    let responder_public_key_hex = hex::encode(responder_public_key);
    let owning_private_wallet_uuid = match &selected_account.scope {
        PublicAccountScope::PrivateWallet { wallet_uuid } => Some(wallet_uuid.clone()),
        PublicAccountScope::Global => None,
    };

    let session_expiry_timestamp =
        now_unix_seconds.saturating_add(WALLETCONNECT_SESSION_EXPIRY_SECS);
    let session = WalletConnectSessionRecord {
        session_uuid: session_uuid.into(),
        pairing_topic: proposal.pairing_topic.clone(),
        session_topic: session_topic.clone(),
        relay_protocol: proposal.relay_protocol.clone(),
        relay_client_id: relay_identity.client_id.clone(),
        peer_metadata: proposal.peer_metadata.clone(),
        approved_namespaces: negotiation.approved_namespaces.clone(),
        selected_public_account_uuid: selected_account.public_account_uuid.clone(),
        selected_public_account_scope: selected_account.scope.clone(),
        owning_private_wallet_uuid,
        keys: WalletConnectSessionKeys {
            sym_key,
            responder_private_key,
            responder_public_key,
        },
        expiry_timestamp: session_expiry_timestamp,
        lifecycle_state: WalletConnectSessionLifecycleState::Active,
    };

    let settle_params = json!({
        "relay": { "protocol": proposal.relay_protocol },
        "controller": {
            "publicKey": responder_public_key_hex,
            "metadata": {
                "name": "RailOxide",
                "description": "RailOxide desktop wallet",
                "url": "https://railgun.org",
                "icons": [],
            },
        },
        "namespaces": session.approved_namespaces,
        "expiry": session.expiry_timestamp,
    });
    let approval_messages = WalletConnectApprovalMessages::new(
        proposal.id,
        proposal.id.saturating_add(1),
        &proposal.relay_protocol,
        &responder_public_key_hex,
        settle_params,
    );
    let relay_steps = approval_messages.encrypted_relay_steps(
        proposal.pairing_topic.clone(),
        pairing_sym_key,
        session_topic,
        &session.keys.sym_key,
        300,
    )?;

    Ok(WalletConnectSessionApproval {
        negotiation,
        session,
        approval_messages,
        relay_steps,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalletConnectProposalRejectionReason {
    UserRejected,
    Expired,
    UnsupportedNamespaces,
}

impl WalletConnectProposalRejectionReason {
    #[must_use]
    pub const fn error_code(self) -> i64 {
        match self {
            Self::UserRejected => 5_000,
            Self::Expired => 8_000,
            Self::UnsupportedNamespaces => 5_100,
        }
    }

    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::UserRejected => "User rejected WalletConnect session proposal",
            Self::Expired => "WalletConnect session proposal expired",
            Self::UnsupportedNamespaces => "Unsupported WalletConnect namespaces",
        }
    }
}

#[must_use]
pub fn reject_walletconnect_session_proposal(
    proposal_id: u64,
    reason: WalletConnectProposalRejectionReason,
) -> WalletConnectJsonRpcResponse<Value> {
    WalletConnectJsonRpcResponse {
        id: proposal_id,
        jsonrpc: "2.0".to_owned(),
        result: None,
        error: Some(WalletConnectJsonRpcError {
            code: reason.error_code(),
            message: reason.message().to_owned(),
        }),
    }
}

fn parse_hex_32(value: &str) -> Result<[u8; 32]> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    let bytes = hex::decode(value).map_err(|_| WalletConnectError::Crypto)?;
    bytes.try_into().map_err(|_| WalletConnectError::Crypto)
}

fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}
