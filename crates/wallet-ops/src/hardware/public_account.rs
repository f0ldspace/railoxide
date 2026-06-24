use alloy::primitives::Address;
use serde::{Deserialize, Serialize};

use super::error::HardwareDerivationError;
use super::path::{HARDENED_BIP32_INDEX, format_bip32_path, hardened_bip32_index};
use super::types::HardwareDeviceKind;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HardwarePublicAccountPathKind {
    LedgerLive,
    TrezorSuite,
    LedgerBip44,
    TrezorBip44,
}

impl HardwarePublicAccountPathKind {
    #[must_use]
    pub const fn device_kind(self) -> HardwareDeviceKind {
        match self {
            Self::LedgerLive | Self::LedgerBip44 => HardwareDeviceKind::Ledger,
            Self::TrezorSuite | Self::TrezorBip44 => HardwareDeviceKind::Trezor,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HardwarePublicAccountDescriptor {
    pub device_kind: HardwareDeviceKind,
    pub path_kind: HardwarePublicAccountPathKind,
    pub path: Vec<u32>,
    #[serde(default)]
    pub wallet_account_index: u32,
    #[serde(default, alias = "account_index")]
    pub public_account_index: u32,
}

impl HardwarePublicAccountDescriptor {
    pub fn for_wallet_public_index(
        device_kind: HardwareDeviceKind,
        wallet_account_index: u32,
        public_account_index: u32,
    ) -> Result<Self, HardwareDerivationError> {
        if wallet_account_index >= HARDENED_BIP32_INDEX {
            return Err(HardwareDerivationError::InvalidDescriptor(
                "hardware public wallet account index is too large",
            ));
        }
        if public_account_index >= HARDENED_BIP32_INDEX {
            return Err(HardwareDerivationError::InvalidDescriptor(
                "hardware public account index is too large",
            ));
        }
        let descriptor = Self::for_wallet_public_index_unchecked(
            device_kind,
            wallet_account_index,
            public_account_index,
        );
        descriptor.validate()?;
        Ok(descriptor)
    }

    pub fn for_device_index(
        device_kind: HardwareDeviceKind,
        account_index: u32,
    ) -> Result<Self, HardwareDerivationError> {
        Self::for_wallet_public_index(device_kind, 0, account_index)
    }

    pub fn validate(&self) -> Result<(), HardwareDerivationError> {
        if self.wallet_account_index >= HARDENED_BIP32_INDEX {
            return Err(HardwareDerivationError::InvalidDescriptor(
                "hardware public wallet account index is too large",
            ));
        }
        if self.public_account_index >= HARDENED_BIP32_INDEX {
            return Err(HardwareDerivationError::InvalidDescriptor(
                "hardware public account index is too large",
            ));
        }
        if self.path_kind.device_kind() != self.device_kind {
            return Err(HardwareDerivationError::InvalidDescriptor(
                "hardware public account path kind does not match device kind",
            ));
        }
        if self.path.len() != 5 {
            return Err(HardwareDerivationError::InvalidDescriptor(
                "hardware public account path must contain 5 segments",
            ));
        }
        let expected = Self::for_wallet_public_index_unchecked(
            self.device_kind,
            self.wallet_account_index,
            self.public_account_index,
        );
        let legacy =
            Self::legacy_for_device_index_unchecked(self.device_kind, self.public_account_index);
        if (self.path_kind != expected.path_kind || self.path != expected.path)
            && (self.path_kind != legacy.path_kind || self.path != legacy.path)
        {
            return Err(HardwareDerivationError::InvalidDescriptor(
                "hardware public account path does not match account index",
            ));
        }
        Ok(())
    }

    #[must_use]
    pub fn path_display(&self) -> String {
        format_bip32_path(&self.path)
    }

    fn for_wallet_public_index_unchecked(
        device_kind: HardwareDeviceKind,
        wallet_account_index: u32,
        public_account_index: u32,
    ) -> Self {
        match device_kind {
            HardwareDeviceKind::Ledger => Self {
                device_kind,
                path_kind: HardwarePublicAccountPathKind::LedgerBip44,
                path: vec![
                    hardened_bip32_index(44),
                    hardened_bip32_index(60),
                    hardened_bip32_index(wallet_account_index),
                    0,
                    public_account_index,
                ],
                wallet_account_index,
                public_account_index,
            },
            HardwareDeviceKind::Trezor => Self {
                device_kind,
                path_kind: HardwarePublicAccountPathKind::TrezorBip44,
                path: vec![
                    hardened_bip32_index(44),
                    hardened_bip32_index(60),
                    hardened_bip32_index(wallet_account_index),
                    0,
                    public_account_index,
                ],
                wallet_account_index,
                public_account_index,
            },
        }
    }

    fn legacy_for_device_index_unchecked(
        device_kind: HardwareDeviceKind,
        account_index: u32,
    ) -> Self {
        match device_kind {
            HardwareDeviceKind::Ledger => Self {
                device_kind,
                path_kind: HardwarePublicAccountPathKind::LedgerLive,
                path: vec![
                    hardened_bip32_index(44),
                    hardened_bip32_index(60),
                    hardened_bip32_index(account_index),
                    0,
                    0,
                ],
                wallet_account_index: 0,
                public_account_index: account_index,
            },
            HardwareDeviceKind::Trezor => Self {
                device_kind,
                path_kind: HardwarePublicAccountPathKind::TrezorSuite,
                path: vec![
                    hardened_bip32_index(44),
                    hardened_bip32_index(60),
                    hardened_bip32_index(0),
                    0,
                    account_index,
                ],
                wallet_account_index: 0,
                public_account_index: account_index,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmedHardwarePublicAccount {
    descriptor: HardwarePublicAccountDescriptor,
    address: Address,
}

impl ConfirmedHardwarePublicAccount {
    #[cfg(any(feature = "hardware", test))]
    pub(super) const fn new(descriptor: HardwarePublicAccountDescriptor, address: Address) -> Self {
        Self {
            descriptor,
            address,
        }
    }

    #[must_use]
    pub const fn descriptor(&self) -> &HardwarePublicAccountDescriptor {
        &self.descriptor
    }

    #[must_use]
    pub const fn address(&self) -> Address {
        self.address
    }

    #[cfg(test)]
    #[must_use]
    pub const fn new_for_tests(
        descriptor: HardwarePublicAccountDescriptor,
        address: Address,
    ) -> Self {
        Self::new(descriptor, address)
    }
}
