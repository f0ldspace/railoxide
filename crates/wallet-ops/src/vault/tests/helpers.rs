use super::super::*;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) const TEST_PASSWORD: &str = "correct horse battery staple";
pub(super) const TEST_WALLET_ID: &str = "wallet-1";
pub(super) const TEST_MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
pub(super) const IMPORT_PRIVATE_KEY_ONE: &str =
    "0x0000000000000000000000000000000000000000000000000000000000000001";
pub(super) const IMPORT_PRIVATE_KEY_TWO: &str =
    "0x0000000000000000000000000000000000000000000000000000000000000002";
pub(super) static TEMP_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(super) fn test_kdf() -> KdfParams {
    KdfParams::new(1024, 1, 1)
}

pub(super) fn temp_db_root() -> PathBuf {
    let dir = std::env::temp_dir().join("railoxide-wallet-vault-tests");
    fs::create_dir_all(&dir).expect("create temp db dir");
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let counter = TEMP_DB_COUNTER.fetch_add(1, Ordering::Relaxed);
    dir.join(format!("db-{pid}-{nanos}-{counter}"))
}

pub(super) fn desktop_store_with_vault() -> (PathBuf, Arc<DbStore>, DesktopVaultStore) {
    let root_dir = temp_db_root();
    let db = Arc::new(
        DbStore::open(DbConfig {
            root_dir: root_dir.clone(),
        })
        .expect("open db"),
    );
    let store = DesktopVaultStore::from_db(Arc::clone(&db));
    let created = create_with_params(TEST_PASSWORD, test_kdf()).expect("create vault");
    store
        .put_metadata(&created.metadata)
        .expect("persist metadata");
    (root_dir, db, store)
}

pub(super) fn import_wallet_with_metadata(
    store: &DesktopVaultStore,
    wallet_id: &str,
    label: &str,
) -> DesktopViewSession {
    let metadata = store
        .new_wallet_metadata(TEST_PASSWORD, wallet_id, 0, WalletSource::Imported, label)
        .expect("wallet metadata");
    store
        .import_wallet_mnemonic_with_metadata(
            TEST_PASSWORD,
            wallet_id,
            0,
            "english",
            TEST_MNEMONIC,
            &metadata,
        )
        .expect("import wallet with metadata");
    store
        .load_view_session(TEST_PASSWORD, wallet_id)
        .expect("load view session")
}

pub(super) fn test_hardware_descriptor(account_index: u32) -> HardwareDerivationDescriptor {
    HardwareDerivationDescriptor::ledger_eip1024_v1(
        crate::hardware::parse_bip32_path("m/44'/60'/0'/0/0").expect("valid path"),
        account_index,
        "ledger-profile-fingerprint".to_owned(),
        crate::hardware::HardwareWalletSyncIntent::CreateNew,
    )
}

pub(super) fn test_trezor_hardware_descriptor(account_index: u32) -> HardwareDerivationDescriptor {
    HardwareDerivationDescriptor::trezor_cipher_key_value_v1(
        crate::hardware::parse_bip32_path("m/44'/60'/0'/0/0").expect("valid path"),
        account_index,
        "trezor-profile-fingerprint".to_owned(),
        crate::hardware::HardwareWalletSyncIntent::CreateNew,
    )
}

pub(super) fn test_hardware_wallet(account_index: u32) -> WalletKeys {
    WalletKeys::from_bip39_entropy(&[42u8; 32], account_index).expect("derive hardware wallet")
}

pub(super) fn test_hardware_receive_address(account_index: u32) -> String {
    test_hardware_wallet(account_index)
        .viewing
        .derive_address(None)
        .expect("derive hardware receive address")
        .to_string()
}

pub(super) fn test_hardware_view_access_key(account_index: u32) -> HardwareViewAccessKey {
    let mut key = [77u8; KEY_LEN];
    key[..4].copy_from_slice(&account_index.to_be_bytes());
    HardwareViewAccessKey::new(key)
}

pub(super) fn load_test_hardware_view_session(
    store: &DesktopVaultStore,
    wallet_id: &str,
    descriptor: &HardwareDerivationDescriptor,
) -> DesktopViewSession {
    let hardware_session = store
        .hardware_profile_session_for_fingerprint(
            TEST_PASSWORD,
            descriptor.device_kind,
            &descriptor.profile_fingerprint,
            None,
        )
        .expect("hardware profile session");
    store
        .load_hardware_view_session(
            TEST_PASSWORD,
            &hardware_session,
            wallet_id,
            &test_hardware_view_access_key(descriptor.account_index),
        )
        .expect("load hardware view session")
}

#[derive(Serialize)]
pub(super) struct LegacyWalletMetadataBundle {
    pub(super) wallet_uuid: String,
    pub(super) label: String,
    pub(super) derivation_index: u32,
}
pub(super) fn test_walletconnect_session(
    session_uuid: &str,
    account: &PublicAccountMetadata,
    relay_client_id: &str,
) -> WalletConnectSessionRecord {
    let mut approved_namespaces = BTreeMap::new();
    approved_namespaces.insert(
        "eip155".to_owned(),
        WalletConnectApprovedNamespace {
            chains: vec!["eip155:1".to_owned()],
            accounts: vec![format!("eip155:1:{}", account.address)],
            methods: vec!["eth_accounts".to_owned(), "eth_requestAccounts".to_owned()],
            events: vec!["accountsChanged".to_owned(), "chainChanged".to_owned()],
        },
    );
    let owning_private_wallet_uuid = match &account.scope {
        PublicAccountScope::PrivateWallet { wallet_uuid } => Some(wallet_uuid.clone()),
        PublicAccountScope::Global => None,
    };

    WalletConnectSessionRecord {
        session_uuid: session_uuid.to_owned(),
        pairing_topic: format!("pairing-{session_uuid}"),
        session_topic: format!("session-{session_uuid}"),
        relay_protocol: crate::WALLETCONNECT_IRN_RELAY_PROTOCOL.to_owned(),
        relay_client_id: relay_client_id.to_owned(),
        peer_metadata: WalletConnectPeerMetadata {
            name: format!("Example Dapp {session_uuid}"),
            description: "Example dapp description".to_owned(),
            url: "https://example.invalid".to_owned(),
            icons: vec!["https://example.invalid/icon.png".to_owned()],
        },
        approved_namespaces,
        selected_public_account_uuid: account.public_account_uuid.clone(),
        selected_public_account_scope: account.scope.clone(),
        owning_private_wallet_uuid,
        keys: WalletConnectSessionKeys {
            sym_key: [1u8; KEY_LEN],
            responder_private_key: [2u8; KEY_LEN],
            responder_public_key: [3u8; KEY_LEN],
        },
        expiry_timestamp: 1_800_000_000,
        lifecycle_state: WalletConnectSessionLifecycleState::Active,
    }
}
pub(super) fn contains_subsequence(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

pub(super) fn railgun_recipient_for_derivation(derivation_index: u32) -> String {
    WalletKeys::from_mnemonic(TEST_MNEMONIC, derivation_index)
        .expect("derive wallet")
        .viewing
        .derive_address(None)
        .expect("derive receive address")
        .to_string()
}
