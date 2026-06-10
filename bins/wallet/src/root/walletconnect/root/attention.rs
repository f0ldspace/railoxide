use super::*;

impl WalletRoot {
    pub(in crate::root) fn sync_walletconnect_attention(&mut self) {
        let next_count = self.walletconnect.attention_count();
        let transition = walletconnect_attention_transition(
            self.walletconnect_attention_count,
            next_count,
            self.walletconnect_window_active,
        );
        if transition.sync_badge_count {
            self.platform_attention.sync_badge_count(next_count);
        }
        if transition.request_attention {
            self.platform_attention.request_attention();
        }
        if transition.clear_attention {
            self.platform_attention.clear_attention();
        }
        self.walletconnect_attention_count = next_count;
    }

    pub(in crate::root) fn sync_walletconnect_attention_for_window(&mut self, window: &Window) {
        self.walletconnect_window_active = window.is_window_active();
        self.sync_walletconnect_attention();
    }
}
