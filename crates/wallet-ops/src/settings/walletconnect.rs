use super::{Deserialize, Serialize, validate_optional_non_empty};
use crate::WALLETCONNECT_DEFAULT_PROJECT_ID;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct WalletConnectSettings {
    pub project_id_override: Option<String>,
}

impl WalletConnectSettings {
    #[must_use]
    pub fn effective_project_id(&self) -> &str {
        self.project_id_override
            .as_deref()
            .map(str::trim)
            .filter(|project_id| !project_id.is_empty())
            .unwrap_or(WALLETCONNECT_DEFAULT_PROJECT_ID)
    }

    pub(super) fn validate(&self, errors: &mut Vec<String>) {
        validate_optional_non_empty(
            "walletconnect.project_id_override",
            self.project_id_override.as_deref(),
            errors,
        );
    }
}
