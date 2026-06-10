use std::fmt;

use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::Zeroizing;

use super::error::HardwareDerivationError;
use super::path::{HARDENED_BIP32_INDEX, format_bip32_path};
use super::types::{HardwareDerivationMethod, HardwareDeviceKind, HardwareWalletSyncIntent};

const HARDWARE_DERIVED_KDF_VERSION_V1: u16 = 1;
const HARDWARE_DERIVED_KDF_SALT_V1: &[u8] = b"railgun:hardware-derived-wallet:kdf:v1";
const HARDWARE_DERIVED_KDF_INFO_PREFIX_V1: &[u8] = b"railgun:hardware-derived-wallet:entropy:v1";
const HARDWARE_VIEW_ACCESS_KDF_INFO_PREFIX_V1: &[u8] =
    b"railgun:hardware-derived-wallet:view-access:v1";

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HardwareDerivationDescriptor {
    pub device_kind: HardwareDeviceKind,
    pub method: HardwareDerivationMethod,
    pub path: Vec<u32>,
    pub account_index: u32,
    pub profile_fingerprint: String,
    pub kdf_version: u16,
    pub sync_intent: HardwareWalletSyncIntent,
}

impl fmt::Debug for HardwareDerivationDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HardwareDerivationDescriptor")
            .field("device_kind", &self.device_kind)
            .field("method", &self.method)
            .field("path", &format_bip32_path(&self.path))
            .field("account_index", &self.account_index)
            .field("profile_fingerprint", &"<redacted>")
            .field("kdf_version", &self.kdf_version)
            .field("sync_intent", &self.sync_intent)
            .finish()
    }
}

impl HardwareDerivationDescriptor {
    #[must_use]
    pub const fn ledger_eip1024_v1(
        path: Vec<u32>,
        account_index: u32,
        profile_fingerprint: String,
        sync_intent: HardwareWalletSyncIntent,
    ) -> Self {
        Self {
            device_kind: HardwareDeviceKind::Ledger,
            method: HardwareDerivationMethod::LedgerEip1024V1,
            path,
            account_index,
            profile_fingerprint,
            kdf_version: HARDWARE_DERIVED_KDF_VERSION_V1,
            sync_intent,
        }
    }

    #[must_use]
    pub const fn trezor_cipher_key_value_v1(
        path: Vec<u32>,
        account_index: u32,
        profile_fingerprint: String,
        sync_intent: HardwareWalletSyncIntent,
    ) -> Self {
        Self {
            device_kind: HardwareDeviceKind::Trezor,
            method: HardwareDerivationMethod::TrezorCipherKeyValueV1,
            path,
            account_index,
            profile_fingerprint,
            kdf_version: HARDWARE_DERIVED_KDF_VERSION_V1,
            sync_intent,
        }
    }

    pub fn validate(&self) -> Result<(), HardwareDerivationError> {
        if self.method.device_kind() != self.device_kind {
            return Err(HardwareDerivationError::InvalidDescriptor(
                "device kind does not match derivation method",
            ));
        }
        if self.account_index >= HARDENED_BIP32_INDEX {
            return Err(HardwareDerivationError::InvalidDescriptor(
                "hardware wallet account index is too large",
            ));
        }
        if self.path.is_empty() {
            return Err(HardwareDerivationError::InvalidDescriptor(
                "derivation path must not be empty",
            ));
        }
        if self.path.len() > 10 {
            return Err(HardwareDerivationError::InvalidDescriptor(
                "derivation path must contain at most 10 segments",
            ));
        }
        if self.profile_fingerprint.trim().is_empty() {
            return Err(HardwareDerivationError::InvalidDescriptor(
                "profile fingerprint must not be empty",
            ));
        }
        if self.kdf_version != HARDWARE_DERIVED_KDF_VERSION_V1 {
            return Err(HardwareDerivationError::UnsupportedKdfVersion(
                self.kdf_version,
            ));
        }
        Ok(())
    }

    #[must_use]
    pub fn kdf_info(&self) -> Vec<u8> {
        let mut info = Vec::with_capacity(
            HARDWARE_DERIVED_KDF_INFO_PREFIX_V1.len()
                + self.path.len() * 4
                + self.profile_fingerprint.len()
                + 64,
        );
        info.extend_from_slice(HARDWARE_DERIVED_KDF_INFO_PREFIX_V1);
        push_kdf_field(&mut info, self.device_kind.as_str().as_bytes());
        push_kdf_field(&mut info, self.method.as_str().as_bytes());
        info.extend_from_slice(&self.kdf_version.to_be_bytes());
        info.extend_from_slice(&self.account_index.to_be_bytes());
        push_kdf_field(&mut info, self.profile_fingerprint.as_bytes());
        let path_len = u8::try_from(self.path.len()).unwrap_or(u8::MAX);
        info.push(path_len);
        for index in &self.path {
            info.extend_from_slice(&index.to_be_bytes());
        }
        info
    }
}

fn push_kdf_field(info: &mut Vec<u8>, field: &[u8]) {
    let len = u16::try_from(field.len()).expect("hardware KDF field length fits in u16");
    info.extend_from_slice(&len.to_be_bytes());
    info.extend_from_slice(field);
}

pub struct HardwareOperationOutput(Zeroizing<[u8; 32]>);

impl HardwareOperationOutput {
    #[must_use]
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    #[must_use]
    pub fn expose_secret(&self) -> &[u8; 32] {
        &self.0
    }

    #[must_use]
    pub fn into_zeroizing(self) -> Zeroizing<[u8; 32]> {
        self.0
    }
}

impl fmt::Debug for HardwareOperationOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HardwareOperationOutput(<redacted>)")
    }
}

pub struct SyntheticRailgunEntropy(Zeroizing<[u8; 32]>);

impl SyntheticRailgunEntropy {
    #[must_use]
    pub fn expose_secret(&self) -> &[u8; 32] {
        &self.0
    }

    #[must_use]
    pub fn into_zeroizing(self) -> Zeroizing<[u8; 32]> {
        self.0
    }
}

pub struct HardwareViewAccessKey(Zeroizing<[u8; 32]>);

impl HardwareViewAccessKey {
    #[must_use]
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    #[must_use]
    pub fn expose_secret(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for HardwareViewAccessKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HardwareViewAccessKey(<redacted>)")
    }
}

impl fmt::Debug for SyntheticRailgunEntropy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SyntheticRailgunEntropy(<redacted>)")
    }
}

pub fn synthetic_entropy_from_hardware_output(
    descriptor: &HardwareDerivationDescriptor,
    output: HardwareOperationOutput,
) -> Result<SyntheticRailgunEntropy, HardwareDerivationError> {
    descriptor.validate()?;
    let output = output.into_zeroizing();
    let hkdf = Hkdf::<Sha256>::new(Some(HARDWARE_DERIVED_KDF_SALT_V1), output.as_ref());
    let mut entropy = [0u8; 32];
    hkdf.expand(&descriptor.kdf_info(), &mut entropy)
        .map_err(|_| HardwareDerivationError::KdfExpand)?;
    Ok(SyntheticRailgunEntropy(Zeroizing::new(entropy)))
}

pub fn hardware_view_access_key_from_hardware_output(
    descriptor: &HardwareDerivationDescriptor,
    output: &HardwareOperationOutput,
) -> Result<HardwareViewAccessKey, HardwareDerivationError> {
    descriptor.validate()?;
    let hkdf = Hkdf::<Sha256>::new(Some(HARDWARE_DERIVED_KDF_SALT_V1), output.expose_secret());
    let mut info = descriptor.kdf_info();
    info.extend_from_slice(HARDWARE_VIEW_ACCESS_KDF_INFO_PREFIX_V1);
    let mut key = [0u8; 32];
    hkdf.expand(&info, &mut key)
        .map_err(|_| HardwareDerivationError::KdfExpand)?;
    Ok(HardwareViewAccessKey::new(key))
}

#[must_use]
pub fn hardware_profile_fingerprint(
    device_kind: HardwareDeviceKind,
    evm_address: impl AsRef<str>,
) -> String {
    format!(
        "{}:evm:{}",
        device_kind.as_str(),
        evm_address.as_ref().to_ascii_lowercase()
    )
}
