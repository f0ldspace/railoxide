use super::super::*;
use super::helpers::*;
use alloy::uint;
use std::fs;

#[test]
fn create_and_unlock_view() {
    let created = create_with_params(TEST_PASSWORD, test_kdf()).expect("create vault");
    let unlocked = unlock_view(&created.metadata, TEST_PASSWORD).expect("unlock view");

    let record = unlocked
        .encrypt_record(
            RecordKind::WalletChainMetadata,
            TEST_WALLET_ID,
            b"chain metadata",
        )
        .expect("encrypt");
    let plaintext = unlocked
        .decrypt_record(RecordKind::WalletChainMetadata, TEST_WALLET_ID, &record)
        .expect("decrypt");

    assert_eq!(&*plaintext, b"chain metadata");
}
#[test]
fn software_spend_signer_requires_valid_grant() {
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
    let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let wallet_id = "spend-signer-wallet";
    store
        .import_wallet_mnemonic(TEST_PASSWORD, wallet_id, 0, "english", mnemonic)
        .expect("import wallet");
    let mut grant = store
        .create_spend_grant(TEST_PASSWORD)
        .expect("create grant");

    let signer = store
        .railgun_spend_signer(&mut grant, wallet_id)
        .expect("load signer");
    let signature = signer.sign_spend_message(uint!(7_U256));

    assert_ne!(signature, [U256::ZERO; 3]);
    assert!(!grant.is_valid());
    assert!(matches!(
        store.railgun_spend_signer(&mut grant, wallet_id),
        Err(VaultError::InvalidSpendGrant)
    ));

    drop(store);
    drop(db);
    fs::remove_dir_all(root_dir).expect("remove temp db dir");
}
#[test]
fn wrong_password_returns_generic_unlock_error() {
    let created = create_with_params(TEST_PASSWORD, test_kdf()).expect("create vault");
    let Err(error) = unlock_view(&created.metadata, "wrong password") else {
        panic!("unlock unexpectedly succeeded");
    };

    assert!(matches!(error, VaultError::UnlockFailed));
}
#[test]
fn tampered_wrapped_key_returns_generic_unlock_error() {
    let created = create_with_params(TEST_PASSWORD, test_kdf()).expect("create vault");
    let mut metadata = created.metadata;
    metadata.wrapped_view_dek.ciphertext[0] ^= 0x01;

    let Err(error) = unlock_view(&metadata, TEST_PASSWORD) else {
        panic!("unlock unexpectedly succeeded");
    };

    assert!(matches!(error, VaultError::UnlockFailed));
}
#[test]
fn view_and_spend_bundles_use_separate_keys() {
    let created = create_with_params(TEST_PASSWORD, test_kdf()).expect("create vault");
    let view_bundle = WalletViewBundle {
        derivation_index: 0,
        spending_public_key: [[1u8; KEY_LEN], [2u8; KEY_LEN]],
        viewing_private_key: [3u8; KEY_LEN],
        viewing_public_key: [4u8; KEY_LEN],
        nullifying_key: [5u8; KEY_LEN],
        master_public_key: [6u8; KEY_LEN],
    };
    let spend_bundle = WalletSpendBundle {
        derivation_index: 0,
        bip39_language: "english".to_string(),
        bip39_entropy: vec![7u8; 32],
    };

    let view_record = created
        .view
        .encrypt_view_bundle(TEST_WALLET_ID, &view_bundle)
        .expect("encrypt view bundle");
    let spend_record = created
        .spend
        .encrypt_spend_bundle(TEST_WALLET_ID, &spend_bundle)
        .expect("encrypt spend bundle");

    assert!(
        created
            .view
            .decrypt_view_bundle(TEST_WALLET_ID, &view_record)
            .is_ok()
    );
    assert!(
        created
            .spend
            .decrypt_spend_bundle(TEST_WALLET_ID, &spend_record)
            .is_ok()
    );
    assert!(
        created
            .view
            .decrypt_record(RecordKind::WalletSpendBundle, TEST_WALLET_ID, &spend_record)
            .is_err()
    );
    assert!(
        created
            .spend
            .decrypt_record(RecordKind::WalletViewBundle, TEST_WALLET_ID, &view_record)
            .is_err()
    );
}
#[test]
fn spend_grant_is_one_use_and_invalidates() {
    let created = create_with_params(TEST_PASSWORD, test_kdf()).expect("create vault");
    let mut grant = create_spend_grant(&created.metadata, TEST_PASSWORD).expect("grant");

    assert_eq!(grant.policy(), SpendGrantPolicy::OneUse);
    assert!(grant.is_valid());
    assert!(grant.spend_unlock().is_ok());

    grant.invalidate();

    assert!(!grant.is_valid());
    assert!(matches!(
        grant.spend_unlock(),
        Err(VaultError::InvalidSpendGrant)
    ));
}
#[test]
fn aad_binds_record_kind_and_id() {
    let created = create_with_params(TEST_PASSWORD, test_kdf()).expect("create vault");
    let record = created
        .view
        .encrypt_record(
            RecordKind::WalletChainMetadata,
            TEST_WALLET_ID,
            b"chain metadata",
        )
        .expect("encrypt");

    assert!(
        created
            .view
            .decrypt_record(RecordKind::WalletCacheRow, TEST_WALLET_ID, &record)
            .is_err()
    );
    assert!(
        created
            .view
            .decrypt_record(RecordKind::WalletChainMetadata, "other-wallet", &record)
            .is_err()
    );
}
