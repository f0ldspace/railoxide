use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum HardwareDeviceKind {
    Ledger,
    Trezor,
}

impl HardwareDeviceKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ledger => "ledger",
            Self::Trezor => "trezor",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum HardwareDerivationMethod {
    LedgerEip1024V1,
    TrezorCipherKeyValueV1,
}

impl HardwareDerivationMethod {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LedgerEip1024V1 => "ledger_eip1024_v1",
            Self::TrezorCipherKeyValueV1 => "trezor_cipher_key_value_v1",
        }
    }

    #[must_use]
    pub const fn device_kind(self) -> HardwareDeviceKind {
        match self {
            Self::LedgerEip1024V1 => HardwareDeviceKind::Ledger,
            Self::TrezorCipherKeyValueV1 => HardwareDeviceKind::Trezor,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HardwareWalletSyncIntent {
    CreateNew,
    RecoverExisting,
}

impl HardwareWalletSyncIntent {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CreateNew => "create_new",
            Self::RecoverExisting => "recover_existing",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardwareTypedDataSigningMode {
    ClearSign,
    Eip712HashFallback,
    Unsupported,
}

impl HardwareTypedDataSigningMode {
    #[must_use]
    pub const fn is_supported(self) -> bool {
        matches!(self, Self::ClearSign | Self::Eip712HashFallback)
    }

    #[must_use]
    pub const fn requires_hash_fallback_warning(self) -> bool {
        matches!(self, Self::Eip712HashFallback)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct HardwareAppVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl HardwareAppVersion {
    #[must_use]
    pub const fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

impl fmt::Display for HardwareAppVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}
