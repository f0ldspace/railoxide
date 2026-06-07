use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::crypto::encode_walletconnect_message;
use super::relay::WalletConnectJsonRpcResponse;
use super::relay::WalletConnectRelayRpc;

pub const WC_SESSION_PROPOSE: &str = "wc_sessionPropose";
pub const WC_SESSION_SETTLE: &str = "wc_sessionSettle";
pub const WC_SESSION_PROPOSE_RESPONSE_TAG: u32 = 1101;
pub const WC_SESSION_SETTLE_REQUEST_TAG: u32 = 1102;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WalletConnectApprovalMessages {
    pub proposal_response: WalletConnectJsonRpcResponse<Value>,
    pub settle_request: super::relay::WalletConnectJsonRpcRequest<Value>,
}

impl WalletConnectApprovalMessages {
    #[must_use]
    pub fn new(
        proposal_id: u64,
        settle_id: u64,
        relay_protocol: &str,
        responder_public_key: &str,
        settle_params: Value,
    ) -> Self {
        Self {
            proposal_response: WalletConnectJsonRpcResponse {
                id: proposal_id,
                jsonrpc: "2.0".to_owned(),
                result: Some(json!({
                    "relay": { "protocol": relay_protocol },
                    "responderPublicKey": responder_public_key,
                })),
                error: None,
            },
            settle_request: super::relay::WalletConnectJsonRpcRequest::new(
                settle_id,
                WC_SESSION_SETTLE,
                settle_params,
            ),
        }
    }

    pub fn encrypted_relay_steps(
        &self,
        pairing_topic: impl Into<String>,
        pairing_sym_key: &[u8; 32],
        session_topic: impl Into<String>,
        session_sym_key: &[u8; 32],
        ttl: u64,
    ) -> super::Result<Vec<WalletConnectRelayStep>> {
        let pairing_topic = pairing_topic.into();
        let session_topic = session_topic.into();
        let proposal_response = serde_json::to_vec(&self.proposal_response)?;
        let settle_request = serde_json::to_vec(&self.settle_request)?;
        let proposal_response =
            encode_walletconnect_message(pairing_sym_key, &proposal_response)?.to_base64();
        let settle_request =
            encode_walletconnect_message(session_sym_key, &settle_request)?.to_base64();
        Ok(vec![
            WalletConnectRelayStep::FetchMessages {
                topic: session_topic.clone(),
            },
            WalletConnectRelayStep::Subscribe {
                topic: session_topic.clone(),
            },
            WalletConnectRelayStep::Publish(WalletConnectRelayRpc::Publish {
                topic: session_topic,
                message: settle_request,
                ttl,
                tag: WC_SESSION_SETTLE_REQUEST_TAG,
            }),
            WalletConnectRelayStep::Publish(WalletConnectRelayRpc::Publish {
                topic: pairing_topic,
                message: proposal_response,
                ttl,
                tag: WC_SESSION_PROPOSE_RESPONSE_TAG,
            }),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalletConnectRelayStep {
    FetchMessages { topic: String },
    Subscribe { topic: String },
    Unsubscribe { topic: String, id: String },
    Publish(WalletConnectRelayRpc),
}

impl WalletConnectRelayStep {
    #[must_use]
    pub fn rpc(&self) -> WalletConnectRelayRpc {
        match self {
            Self::FetchMessages { topic } => WalletConnectRelayRpc::FetchMessages {
                topic: topic.clone(),
            },
            Self::Subscribe { topic } => WalletConnectRelayRpc::Subscribe {
                topic: topic.clone(),
            },
            Self::Unsubscribe { topic, id } => WalletConnectRelayRpc::Unsubscribe {
                topic: topic.clone(),
                id: id.clone(),
            },
            Self::Publish(rpc) => rpc.clone(),
        }
    }
}
