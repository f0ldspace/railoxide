use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::vault::WalletConnectSessionRecord;

use super::Result;
use super::crypto::encode_walletconnect_message;
use super::relay::{WalletConnectJsonRpcRequest, WalletConnectRelayRpc};
use super::session::WalletConnectRelayStep;

const WC_SESSION_DELETE: &str = "wc_sessionDelete";
const WC_SESSION_DELETE_REQUEST_TAG: u32 = 1112;
const WC_SESSION_DELETE_TTL_SECS: u64 = 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalletConnectTerminalLifecycleEnd {
    Disconnect,
    Expiry,
    Invalidation,
    Revoke,
    InboundDelete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SubscribedTopic {
    subscription_id: String,
    persisted_session: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WalletConnectRelayLifecycle {
    local_processing_active: bool,
    subscribed_topics: BTreeMap<String, SubscribedTopic>,
}

impl WalletConnectRelayLifecycle {
    #[must_use]
    pub const fn local_processing_active(&self) -> bool {
        self.local_processing_active
    }

    pub fn add_subscribed_topic(
        &mut self,
        topic: impl Into<String>,
        subscription_id: impl Into<String>,
        persisted_session: bool,
    ) {
        self.subscribed_topics.insert(
            topic.into(),
            SubscribedTopic {
                subscription_id: subscription_id.into(),
                persisted_session,
            },
        );
        self.local_processing_active = true;
    }

    pub const fn pause_for_lock_or_shutdown(&mut self) -> Vec<WalletConnectRelayRpc> {
        self.local_processing_active = false;
        Vec::new()
    }

    pub fn resume_after_unlock(&mut self) {
        self.local_processing_active = !self.subscribed_topics.is_empty();
    }

    #[must_use]
    pub fn reconnect_steps(&self) -> Vec<WalletConnectRelayStep> {
        self.subscribed_topics
            .keys()
            .flat_map(|topic| Self::restored_session_steps(topic.clone()))
            .collect()
    }

    pub fn terminal_end(
        &mut self,
        topic: &str,
        _reason: WalletConnectTerminalLifecycleEnd,
    ) -> Option<WalletConnectRelayRpc> {
        let subscribed = self.subscribed_topics.remove(topic)?;
        self.local_processing_active = !self.subscribed_topics.is_empty();
        Some(WalletConnectRelayRpc::Unsubscribe {
            topic: topic.to_owned(),
            id: subscribed.subscription_id,
        })
    }

    #[must_use]
    pub fn restored_session_steps(topic: impl Into<String>) -> Vec<WalletConnectRelayStep> {
        let topic = topic.into();
        vec![
            WalletConnectRelayStep::FetchMessages {
                topic: topic.clone(),
            },
            WalletConnectRelayStep::Subscribe { topic },
        ]
    }

    #[must_use]
    pub fn immediate_fetch_step(topic: impl Into<String>) -> WalletConnectRelayStep {
        WalletConnectRelayStep::FetchMessages {
            topic: topic.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletConnectDisconnectPlan {
    pub delete_request: WalletConnectJsonRpcRequest<Value>,
    pub relay_steps: Vec<WalletConnectRelayStep>,
}

pub fn build_walletconnect_disconnect_plan(
    session: &WalletConnectSessionRecord,
    request_id: u64,
    subscription_id: Option<&str>,
) -> Result<WalletConnectDisconnectPlan> {
    let delete_request = WalletConnectJsonRpcRequest::new(
        request_id,
        WC_SESSION_DELETE,
        json!({
            "code": 6_000,
            "message": "User disconnected WalletConnect session",
        }),
    );
    let delete_message =
        encode_walletconnect_message(&session.keys.sym_key, &serde_json::to_vec(&delete_request)?)?
            .to_base64();
    let mut relay_steps = vec![WalletConnectRelayStep::Publish(
        WalletConnectRelayRpc::Publish {
            topic: session.session_topic.clone(),
            message: delete_message,
            ttl: WC_SESSION_DELETE_TTL_SECS,
            tag: WC_SESSION_DELETE_REQUEST_TAG,
        },
    )];
    if let Some(subscription_id) = subscription_id {
        relay_steps.push(WalletConnectRelayStep::Unsubscribe {
            topic: session.session_topic.clone(),
            id: subscription_id.to_owned(),
        });
    }
    Ok(WalletConnectDisconnectPlan {
        delete_request,
        relay_steps,
    })
}
