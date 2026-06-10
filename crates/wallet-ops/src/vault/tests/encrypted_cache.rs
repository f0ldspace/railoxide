use super::super::*;
use super::helpers::*;
use alloy::uint;
use std::fs;

#[test]
fn cache_row_ids_are_deterministic_and_context_bound() {
    let created = create_with_params(TEST_PASSWORD, test_kdf()).expect("create vault");
    let cache_keys = created
        .view
        .derive_cache_keys("opaque-wallet-chain-a")
        .expect("cache keys");
    let other_cache_keys = created
        .view
        .derive_cache_keys("opaque-wallet-chain-b")
        .expect("other cache keys");

    let row_id = cache_keys.row_id(4, 42, b"stable-utxo");
    let same_row_id = cache_keys.row_id(4, 42, b"stable-utxo");
    let other_position = cache_keys.row_id(4, 43, b"stable-utxo");
    let other_namespace = other_cache_keys.row_id(4, 42, b"stable-utxo");

    assert_eq!(row_id, same_row_id);
    assert_ne!(row_id, other_position);
    assert_ne!(row_id, other_namespace);
    assert_eq!(CacheKeys::row_record_id(&row_id).len(), 64);
}
#[test]
fn encrypted_cache_rows_are_bound_to_opaque_row_id() {
    let created = create_with_params(TEST_PASSWORD, test_kdf()).expect("create vault");
    let cache_keys = created
        .view
        .derive_cache_keys("opaque-wallet-chain")
        .expect("cache keys");
    let row_id = cache_keys.row_id(4, 42, b"stable-utxo");
    let other_row_id = cache_keys.row_id(4, 43, b"stable-utxo");
    let record = cache_keys
        .encrypt_row(&row_id, b"private utxo payload")
        .expect("encrypt row");
    let mut tampered = record.clone();
    tampered.ciphertext[0] ^= 0x01;

    let plaintext = cache_keys
        .decrypt_row(&row_id, &record)
        .expect("decrypt row");
    assert_eq!(&*plaintext, b"private utxo payload");
    assert!(cache_keys.decrypt_row(&other_row_id, &record).is_err());
    assert!(cache_keys.decrypt_row(&row_id, &tampered).is_err());
}
#[test]
fn encrypted_cache_store_hides_wallet_history_details() {
    use alloy::primitives::{FixedBytes, U256};
    use railgun_wallet::{Note, Utxo, UtxoCommitmentKind, UtxoSource};
    use sync_service::WalletCacheStore;

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
    let wallet_id = "encrypted-cache-wallet";
    store
        .import_wallet_mnemonic(TEST_PASSWORD, wallet_id, 0, "english", mnemonic)
        .expect("import wallet");
    let view_session = Arc::new(
        store
            .load_view_session(TEST_PASSWORD, wallet_id)
            .expect("load view session"),
    );
    let chain_metadata = store
        .wallet_chain_metadata_for_session(
            view_session.as_ref(),
            0,
            1,
            "0x1111111111111111111111111111111111111111",
            100,
        )
        .expect("chain metadata");
    let wallet_chain_uuid = chain_metadata.wallet_chain_uuid.clone();
    let cache_store = DesktopEncryptedWalletCacheStore::new(
        Arc::clone(&db),
        Arc::clone(&view_session),
        chain_metadata,
    )
    .expect("encrypted cache store");
    let wallet_utxo = WalletUtxo {
        utxo: Utxo::new(
            Note {
                token_hash: U256::from_be_bytes([0x44; KEY_LEN]),
                value: U256::from_be_bytes([0x55; KEY_LEN]),
                random: [0x66; 16],
                npk: U256::from_be_bytes([0x77; KEY_LEN]),
            },
            7,
            42,
            UtxoSource {
                tx_hash: FixedBytes::from([0x88; KEY_LEN]),
                block_number: 123,
                block_timestamp: 1_700_000_123,
            },
            UtxoCommitmentKind::Transact,
        ),
        spent: Some(UtxoSource {
            tx_hash: FixedBytes::from([0x99; KEY_LEN]),
            block_number: 124,
            block_timestamp: 1_700_000_124,
        }),
    };

    cache_store
        .store_wallet_utxos(
            "ignored-cache-key",
            std::slice::from_ref(&wallet_utxo),
            Some(150),
            Some([0xaa; KEY_LEN]),
        )
        .expect("store encrypted cache");
    let rows = db
        .list_desktop_wallet_vault_records(&wallet_cache_row_prefix(&wallet_chain_uuid))
        .expect("list encrypted cache rows");
    let chain_payload = db
        .get_desktop_wallet_vault_record(&wallet_chain_metadata_record_key(&wallet_chain_uuid))
        .expect("load chain metadata record")
        .expect("chain metadata present");
    let loaded = cache_store
        .load_wallet_utxos("ignored-cache-key")
        .expect("load encrypted cache");
    let loaded_meta = store
        .load_wallet_chain_metadata(TEST_PASSWORD, &wallet_chain_uuid)
        .expect("load updated chain metadata");

    assert_eq!(rows.len(), 1);
    assert_eq!(loaded.len(), 1);
    assert_eq!(
        loaded[0].utxo.note.token_hash,
        wallet_utxo.utxo.note.token_hash
    );
    assert_eq!(
        loaded[0].utxo.source.tx_hash,
        wallet_utxo.utxo.source.tx_hash
    );
    assert_eq!(loaded[0].spent, wallet_utxo.spent);
    assert_eq!(loaded_meta.last_scanned_block, 150);
    assert_eq!(loaded_meta.last_scanned_block_hash, Some([0xaa; KEY_LEN]));

    let row_key = rows[0].key.as_bytes();
    let row_payload = &rows[0].payload;
    assert!(!contains_subsequence(row_key, b"1111111111111111"));
    assert!(!contains_subsequence(row_payload, &[0x44; KEY_LEN]));
    assert!(!contains_subsequence(row_payload, &[0x55; KEY_LEN]));
    assert!(!contains_subsequence(row_payload, &[0x66; 16]));
    assert!(!contains_subsequence(row_payload, &[0x77; KEY_LEN]));
    assert!(!contains_subsequence(row_payload, &[0x88; KEY_LEN]));
    assert!(!contains_subsequence(row_payload, &[0x99; KEY_LEN]));
    assert!(!contains_subsequence(&chain_payload, b"1111111111111111"));

    drop(store);
    drop(db);
    fs::remove_dir_all(root_dir).expect("remove temp db dir");
}
#[test]
fn encrypted_cache_upsert_does_not_delete_existing_rows() {
    use alloy::primitives::{FixedBytes, U256};
    use railgun_wallet::{Note, Utxo, UtxoCommitmentKind, UtxoSource};
    use sync_service::WalletCacheStore;

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
    let wallet_id = "encrypted-cache-upsert-wallet";
    let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    store
        .import_wallet_mnemonic(TEST_PASSWORD, wallet_id, 0, "english", mnemonic)
        .expect("import wallet");
    let view_session = Arc::new(
        store
            .load_view_session(TEST_PASSWORD, wallet_id)
            .expect("load view session"),
    );
    let mut chain_metadata = store
        .wallet_chain_metadata_for_session(
            view_session.as_ref(),
            0,
            1,
            "0x1111111111111111111111111111111111111111",
            100,
        )
        .expect("chain metadata");
    let wallet_chain_uuid = chain_metadata.wallet_chain_uuid.clone();
    let cache_store = DesktopEncryptedWalletCacheStore::new(
        Arc::clone(&db),
        Arc::clone(&view_session),
        chain_metadata.clone(),
    )
    .expect("encrypted cache store");
    let first = WalletUtxo {
        utxo: Utxo::new(
            Note {
                token_hash: U256::from_be_bytes([0x11; KEY_LEN]),
                value: uint!(1_U256),
                random: [0x22; 16],
                npk: U256::from_be_bytes([0x33; KEY_LEN]),
            },
            3,
            1,
            UtxoSource {
                tx_hash: FixedBytes::from([0x44; KEY_LEN]),
                block_number: 101,
                block_timestamp: 1_700_000_101,
            },
            UtxoCommitmentKind::Transact,
        ),
        spent: None,
    };
    let mut second = first.clone();
    second.utxo.position = 2;
    second.utxo.source.tx_hash = FixedBytes::from([0x55; KEY_LEN]);
    let mut rewound_source = first.clone();
    rewound_source.utxo.position = 3;
    rewound_source.utxo.source = UtxoSource {
        tx_hash: FixedBytes::from([0x66; KEY_LEN]),
        block_number: 170,
        block_timestamp: 1_700_000_170,
    };
    let mut rewound_spend = first.clone();
    rewound_spend.utxo.position = 4;
    rewound_spend.utxo.source.tx_hash = FixedBytes::from([0x77; KEY_LEN]);
    rewound_spend.spent = Some(UtxoSource {
        tx_hash: FixedBytes::from([0x88; KEY_LEN]),
        block_number: 170,
        block_timestamp: 1_700_000_170,
    });

    cache_store
        .store_wallet_utxos(
            "ignored",
            &[first.clone(), second, rewound_source, rewound_spend],
            Some(110),
            None,
        )
        .expect("store full cache");
    cache_store
        .store_wallet_utxos("ignored", std::slice::from_ref(&first), Some(120), None)
        .expect("upsert partial cache");
    let loaded = cache_store
        .load_wallet_utxos("ignored")
        .expect("load upserted cache");
    assert_eq!(loaded.len(), 4);
    assert!(loaded.iter().any(|utxo| utxo.utxo.position == 1));
    assert!(loaded.iter().any(|utxo| utxo.utxo.position == 2));
    assert!(loaded.iter().any(|utxo| utxo.utxo.position == 3));
    assert!(loaded.iter().any(|utxo| utxo.utxo.position == 4));

    store
        .rewind_wallet_chain_cache_with_session(view_session.as_ref(), &mut chain_metadata, 150)
        .expect("rewind encrypted cache");
    let loaded = cache_store
        .load_wallet_utxos("ignored")
        .expect("load rewound cache");
    assert_eq!(loaded.len(), 3);
    assert!(loaded.iter().any(|utxo| utxo.utxo.position == 1));
    assert!(loaded.iter().any(|utxo| utxo.utxo.position == 2));
    assert!(!loaded.iter().any(|utxo| utxo.utxo.position == 3));
    assert!(
        loaded
            .iter()
            .any(|utxo| utxo.utxo.position == 4 && utxo.spent.is_none())
    );
    let metadata = store
        .load_wallet_chain_metadata(TEST_PASSWORD, &wallet_chain_uuid)
        .expect("load rewound metadata");
    assert_eq!(metadata.last_scanned_block, 149);
    assert_eq!(metadata.last_scanned_block_hash, None);

    cache_store
        .replace_wallet_cache("ignored", std::slice::from_ref(&first), 160, None)
        .expect("replace encrypted cache");
    let loaded = cache_store
        .load_wallet_utxos("ignored")
        .expect("load replaced cache");
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].utxo.position, 1);
    let metadata = store
        .load_wallet_chain_metadata(TEST_PASSWORD, &wallet_chain_uuid)
        .expect("load replaced metadata");
    assert_eq!(metadata.last_scanned_block, 160);
    assert_eq!(metadata.last_scanned_block_hash, None);

    store
        .reset_wallet_chain_cache_with_session(view_session.as_ref(), &mut chain_metadata, 160)
        .expect("reset encrypted cache");
    assert!(
        cache_store
            .load_wallet_utxos("ignored")
            .expect("load reset cache")
            .is_empty()
    );
    let metadata = store
        .load_wallet_chain_metadata(TEST_PASSWORD, &wallet_chain_uuid)
        .expect("load reset metadata");
    assert_eq!(metadata.last_scanned_block, 159);
    assert_eq!(metadata.last_scanned_block_hash, None);

    drop(store);
    drop(db);
    fs::remove_dir_all(root_dir).expect("remove temp db dir");
}
