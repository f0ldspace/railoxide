use super::*;

#[test]
fn walletconnect_attention_count_counts_proposal_and_requests() {
    assert_eq!(walletconnect_attention_count(false, 0), 0);
    assert_eq!(walletconnect_attention_count(true, 0), 1);
    assert_eq!(walletconnect_attention_count(false, 2), 2);
    assert_eq!(walletconnect_attention_count(true, 2), 3);
}

#[test]
fn walletconnect_attention_transition_suppresses_active_window_attention() {
    assert_eq!(
        walletconnect_attention_transition(0, 1, true),
        WalletConnectAttentionTransition {
            sync_badge_count: true,
            request_attention: false,
            clear_attention: false,
        }
    );
}

#[test]
fn walletconnect_attention_transition_requests_inactive_increases() {
    assert_eq!(
        walletconnect_attention_transition(1, 2, false),
        WalletConnectAttentionTransition {
            sync_badge_count: true,
            request_attention: true,
            clear_attention: false,
        }
    );
}

#[test]
fn walletconnect_attention_transition_syncs_count_decreases_without_request() {
    assert_eq!(
        walletconnect_attention_transition(3, 1, false),
        WalletConnectAttentionTransition {
            sync_badge_count: true,
            request_attention: false,
            clear_attention: false,
        }
    );
}

#[test]
fn walletconnect_attention_transition_clears_zero_count() {
    assert_eq!(
        walletconnect_attention_transition(1, 0, false),
        WalletConnectAttentionTransition {
            sync_badge_count: true,
            request_attention: false,
            clear_attention: true,
        }
    );
}

#[test]
fn walletconnect_attention_transition_ignores_unchanged_count() {
    assert_eq!(
        walletconnect_attention_transition(2, 2, false),
        WalletConnectAttentionTransition {
            sync_badge_count: false,
            request_attention: false,
            clear_attention: false,
        }
    );
}
