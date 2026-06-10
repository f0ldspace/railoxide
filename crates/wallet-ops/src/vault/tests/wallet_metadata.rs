use super::super::*;
use super::helpers::*;
use std::fs;

#[test]
fn wallet_metadata_flows_auto_create_initial_public_account() {
    let (root_dir, db, store) = desktop_store_with_vault();
    let generated_seed = generate_seed_material().expect("generate seed");
    let generated_wallet_id = "generated-public-wallet";
    let generated_metadata = store
        .new_wallet_metadata(
            TEST_PASSWORD,
            generated_wallet_id,
            0,
            WalletSource::Generated,
            "Generated",
        )
        .expect("generated wallet metadata");
    store
        .store_generated_wallet_with_metadata(
            TEST_PASSWORD,
            generated_wallet_id,
            0,
            "english",
            &generated_seed,
            &generated_metadata,
        )
        .expect("store generated wallet with metadata");
    let generated_session = store
        .load_view_session(TEST_PASSWORD, generated_wallet_id)
        .expect("generated view session");
    let generated_accounts = store
        .list_active_public_accounts_for_session(&generated_session)
        .expect("generated public accounts");
    assert_eq!(generated_accounts.len(), 1);
    assert_eq!(generated_accounts[0].source, PublicAccountSource::Derived);
    assert_eq!(generated_accounts[0].label.as_deref(), Some("Account #1"));
    assert_eq!(generated_accounts[0].derivation_index, Some(0));
    assert_eq!(
        generated_accounts[0].address,
        derive_public_evm_address_from_entropy(generated_seed.entropy.as_slice(), 0)
            .expect("generated public address")
    );

    let imported_wallet_id = "imported-public-wallet";
    let imported_metadata = store
        .new_wallet_metadata(
            TEST_PASSWORD,
            imported_wallet_id,
            0,
            WalletSource::Imported,
            "Imported",
        )
        .expect("imported wallet metadata");
    store
        .import_wallet_mnemonic_with_metadata(
            TEST_PASSWORD,
            imported_wallet_id,
            0,
            "english",
            TEST_MNEMONIC,
            &imported_metadata,
        )
        .expect("import wallet with metadata");
    let imported_session = store
        .load_view_session(TEST_PASSWORD, imported_wallet_id)
        .expect("imported view session");
    let imported_accounts = store
        .list_active_public_accounts_for_session(&imported_session)
        .expect("imported public accounts");
    let imported_entropy = bip39_entropy_from_mnemonic(TEST_MNEMONIC).expect("mnemonic entropy");
    assert_eq!(imported_accounts.len(), 1);
    assert_eq!(imported_accounts[0].source, PublicAccountSource::Derived);
    assert_eq!(imported_accounts[0].label.as_deref(), Some("Account #1"));
    assert_eq!(imported_accounts[0].derivation_index, Some(0));
    assert_eq!(
        imported_accounts[0].address,
        derive_public_evm_address_from_entropy(&imported_entropy, 0)
            .expect("imported public address")
    );

    let legacy_wallet_id = "metadata-less-public-wallet";
    store
        .import_wallet_mnemonic(TEST_PASSWORD, legacy_wallet_id, 0, "english", TEST_MNEMONIC)
        .expect("import metadata-less wallet");
    let legacy_session = store
        .load_view_session(TEST_PASSWORD, legacy_wallet_id)
        .expect("legacy view session");
    assert!(
        store
            .list_active_public_accounts_for_session(&legacy_session)
            .expect("legacy public accounts")
            .is_empty()
    );

    drop(store);
    drop(db);
    fs::remove_dir_all(root_dir).expect("remove temp db dir");
}
#[test]
fn wallet_metadata_listing_defaults_and_synthesizes_records() {
    let (root_dir, db, store) = desktop_store_with_vault();
    let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let legacy_wallet_id = "legacy-wallet";
    let missing_wallet_id = "missing-metadata-wallet";
    store
        .import_wallet_mnemonic(TEST_PASSWORD, legacy_wallet_id, 0, "english", mnemonic)
        .expect("import legacy wallet");
    store
        .import_wallet_mnemonic(TEST_PASSWORD, missing_wallet_id, 1, "english", mnemonic)
        .expect("import metadata-less wallet");

    let legacy = LegacyWalletMetadataBundle {
        wallet_uuid: legacy_wallet_id.to_string(),
        label: "Legacy wallet".to_string(),
        derivation_index: 0,
    };
    let view = store.unlock_view(TEST_PASSWORD).expect("unlock view");
    let record = encrypt_serialized(
        view.view_dek(),
        RecordKind::WalletMetadata,
        legacy_wallet_id,
        &legacy,
    )
    .expect("encrypt legacy metadata");
    let (key, payload) = record
        .to_record_entry(wallet_metadata_record_key(legacy_wallet_id))
        .expect("encode legacy metadata");
    db.put_desktop_wallet_vault_records(&[(key, payload)])
        .expect("store legacy metadata");

    let metadata = store
        .list_wallet_metadata(TEST_PASSWORD)
        .expect("list wallet metadata");
    let legacy = metadata
        .iter()
        .find(|metadata| metadata.wallet_uuid == legacy_wallet_id)
        .expect("legacy metadata");
    let synthesized = metadata
        .iter()
        .find(|metadata| metadata.wallet_uuid == missing_wallet_id)
        .expect("synthesized metadata");

    assert_eq!(metadata.len(), 2);
    assert_eq!(legacy.status, WalletStatus::Active);
    assert_eq!(legacy.display_order, 0);
    assert_eq!(synthesized.label, "Wallet 2");
    assert_eq!(synthesized.derivation_index, 1);
    assert_eq!(synthesized.status, WalletStatus::Active);
    assert_eq!(synthesized.display_order, 1);

    let persisted_legacy = store
        .load_wallet_metadata(TEST_PASSWORD, legacy_wallet_id)
        .expect("load persisted legacy metadata");
    let persisted_synthesized = store
        .load_wallet_metadata(TEST_PASSWORD, missing_wallet_id)
        .expect("load synthesized metadata");
    assert_eq!(persisted_legacy, legacy.clone());
    assert_eq!(persisted_synthesized, synthesized.clone());

    drop(store);
    drop(db);
    fs::remove_dir_all(root_dir).expect("remove temp db dir");
}
#[test]
fn wallet_label_validation_defaults_update_reorder_and_deactivate() {
    let (root_dir, db, store) = desktop_store_with_vault();
    assert_eq!(
        store
            .default_wallet_label(TEST_PASSWORD)
            .expect("default label"),
        PRIMARY_WALLET_LABEL
    );

    let seed = generate_seed_material().expect("generate seed");
    let first_wallet_id = "first-wallet";
    let first_metadata = store
        .new_wallet_metadata(
            TEST_PASSWORD,
            first_wallet_id,
            0,
            WalletSource::Generated,
            "  Primary wallet  ",
        )
        .expect("first wallet metadata");
    assert_eq!(first_metadata.label, PRIMARY_WALLET_LABEL);
    assert_eq!(first_metadata.display_order, 0);
    store
        .store_generated_wallet_with_metadata(
            TEST_PASSWORD,
            first_wallet_id,
            0,
            "english",
            &seed,
            &first_metadata,
        )
        .expect("store first wallet");
    assert_eq!(
        store
            .default_wallet_label(TEST_PASSWORD)
            .expect("second default label"),
        "Wallet 2"
    );
    assert!(matches!(
        store.new_wallet_metadata(
            TEST_PASSWORD,
            "duplicate",
            0,
            WalletSource::Imported,
            "primary wallet",
        ),
        Err(VaultError::DuplicateWalletLabel)
    ));
    assert!(matches!(
        store.new_wallet_metadata(TEST_PASSWORD, "empty", 0, WalletSource::Imported, "   "),
        Err(VaultError::InvalidWalletLabel)
    ));
    assert!(matches!(
        store.preflight_new_wallet_metadata(TEST_PASSWORD, "primary wallet"),
        Err(VaultError::DuplicateWalletLabel)
    ));
    assert!(matches!(
        store.preflight_new_wallet_metadata(TEST_PASSWORD, "   "),
        Err(VaultError::InvalidWalletLabel)
    ));
    assert_eq!(
        store
            .preflight_new_wallet_metadata(TEST_PASSWORD, "  Wallet 2  ")
            .expect("preflight new label"),
        "Wallet 2"
    );

    let second_wallet_id = "second-wallet";
    let second_metadata = store
        .new_wallet_metadata(
            TEST_PASSWORD,
            second_wallet_id,
            0,
            WalletSource::Generated,
            "Wallet 2",
        )
        .expect("second wallet metadata");
    store
        .store_generated_wallet_with_metadata(
            TEST_PASSWORD,
            second_wallet_id,
            0,
            "english",
            &seed,
            &second_metadata,
        )
        .expect("store second wallet");

    let updated = store
        .update_wallet_label(TEST_PASSWORD, first_wallet_id, "  Main  ")
        .expect("update label");
    assert_eq!(updated.label, "Main");
    assert_eq!(updated.wallet_uuid, first_wallet_id);
    assert_eq!(updated.status, WalletStatus::Active);
    assert_eq!(updated.display_order, 0);
    assert!(matches!(
        store.update_wallet_label(TEST_PASSWORD, second_wallet_id, "main"),
        Err(VaultError::DuplicateWalletLabel)
    ));

    let reordered = store
        .reorder_active_wallets(
            TEST_PASSWORD,
            &[second_wallet_id.to_string(), first_wallet_id.to_string()],
        )
        .expect("reorder active wallets");
    assert_eq!(reordered[0].wallet_uuid, second_wallet_id);
    assert_eq!(reordered[0].display_order, 0);
    assert_eq!(reordered[1].wallet_uuid, first_wallet_id);
    assert_eq!(reordered[1].display_order, 1);
    assert!(matches!(
        store.reorder_active_wallets(TEST_PASSWORD, &[first_wallet_id.to_string()]),
        Err(VaultError::InvalidWalletOrder)
    ));

    let deactivated = store
        .deactivate_wallet(TEST_PASSWORD, second_wallet_id)
        .expect("deactivate second wallet");
    assert_eq!(deactivated.status, WalletStatus::Inactive);
    let active = store
        .active_wallet_metadata(TEST_PASSWORD)
        .expect("active metadata");
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].wallet_uuid, first_wallet_id);
    assert!(
        store
            .load_view_session(TEST_PASSWORD, second_wallet_id)
            .is_ok()
    );
    assert!(matches!(
        store.deactivate_wallet(TEST_PASSWORD, first_wallet_id),
        Err(VaultError::LastActiveWallet)
    ));

    drop(store);
    drop(db);
    fs::remove_dir_all(root_dir).expect("remove temp db dir");
}
#[test]
fn session_wallet_management_renames_hides_shows_reorders_and_guards_last_active() {
    let (root_dir, db, store) = desktop_store_with_vault();
    let first_wallet_id = "session-wallet-a";
    let second_wallet_id = "session-wallet-b";
    let third_wallet_id = "session-wallet-c";
    let first_session = import_wallet_with_metadata(&store, first_wallet_id, "Alpha");
    let _second_session = import_wallet_with_metadata(&store, second_wallet_id, "Beta");
    let _third_session = import_wallet_with_metadata(&store, third_wallet_id, "Gamma");

    let metadata = store
        .list_wallet_metadata_for_session(&first_session, true)
        .expect("list all wallet metadata");
    assert_eq!(metadata.len(), 3);
    assert_eq!(
        metadata
            .iter()
            .map(|metadata| metadata.wallet_uuid.as_str())
            .collect::<Vec<_>>(),
        vec![first_wallet_id, second_wallet_id, third_wallet_id]
    );
    assert_eq!(
        store
            .list_wallet_metadata_for_session(&first_session, false)
            .expect("list active wallet metadata")
            .len(),
        3
    );

    let updated = store
        .update_wallet_label_for_session(&first_session, second_wallet_id, "  Main  ")
        .expect("rename wallet");
    assert_eq!(updated.label, "Main");
    assert!(matches!(
        store.update_wallet_label_for_session(&first_session, third_wallet_id, "alpha"),
        Err(VaultError::DuplicateWalletLabel)
    ));
    assert!(matches!(
        store.update_wallet_label_for_session(&first_session, third_wallet_id, "   "),
        Err(VaultError::InvalidWalletLabel)
    ));

    let hidden = store
        .set_wallet_active_for_session(&first_session, second_wallet_id, false)
        .expect("hide wallet");
    assert_eq!(hidden.status, WalletStatus::Inactive);
    let active = store
        .list_wallet_metadata_for_session(&first_session, false)
        .expect("list active after hide");
    assert_eq!(
        active
            .iter()
            .map(|metadata| metadata.wallet_uuid.as_str())
            .collect::<Vec<_>>(),
        vec![first_wallet_id, third_wallet_id]
    );
    assert!(
        store
            .load_view_session(TEST_PASSWORD, second_wallet_id)
            .is_ok()
    );

    let shown = store
        .set_wallet_active_for_session(&first_session, second_wallet_id, true)
        .expect("show wallet");
    assert_eq!(shown.status, WalletStatus::Active);
    let active = store
        .list_wallet_metadata_for_session(&first_session, false)
        .expect("list active after show");
    assert_eq!(
        active
            .iter()
            .map(|metadata| metadata.wallet_uuid.as_str())
            .collect::<Vec<_>>(),
        vec![first_wallet_id, third_wallet_id, second_wallet_id]
    );

    let reordered = store
        .reorder_active_wallets_for_session(
            &first_session,
            &[
                second_wallet_id.to_string(),
                first_wallet_id.to_string(),
                third_wallet_id.to_string(),
            ],
        )
        .expect("reorder active wallets");
    assert_eq!(reordered[0].wallet_uuid, second_wallet_id);
    assert_eq!(reordered[0].display_order, 0);
    assert_eq!(reordered[1].wallet_uuid, first_wallet_id);
    assert_eq!(reordered[1].display_order, 1);
    assert_eq!(reordered[2].wallet_uuid, third_wallet_id);
    assert_eq!(reordered[2].display_order, 2);
    assert!(matches!(
        store.reorder_active_wallets_for_session(&first_session, &[first_wallet_id.to_string()]),
        Err(VaultError::InvalidWalletOrder)
    ));

    store
        .set_wallet_active_for_session(&first_session, first_wallet_id, false)
        .expect("hide first wallet");
    store
        .set_wallet_active_for_session(&first_session, second_wallet_id, false)
        .expect("hide second wallet");
    assert!(matches!(
        store.set_wallet_active_for_session(&first_session, third_wallet_id, false),
        Err(VaultError::LastActiveWallet)
    ));

    drop(store);
    drop(db);
    fs::remove_dir_all(root_dir).expect("remove temp db dir");
}
#[test]
fn permanent_wallet_delete_purges_wallet_scoped_records_and_guards_last_active() {
    let (root_dir, db, store) = desktop_store_with_vault();
    let first_wallet_id = "delete-wallet-a";
    let second_wallet_id = "delete-wallet-b";
    let third_wallet_id = "delete-wallet-c";
    let first_session = import_wallet_with_metadata(&store, first_wallet_id, "Alpha");
    let second_session = import_wallet_with_metadata(&store, second_wallet_id, "Beta");
    let _third_session = import_wallet_with_metadata(&store, third_wallet_id, "Gamma");
    let first_chain = store
        .wallet_chain_metadata_for_session(
            &first_session,
            0,
            1,
            "0x1111111111111111111111111111111111111111",
            100,
        )
        .expect("first chain metadata");
    let second_chain = store
        .wallet_chain_metadata_for_session(
            &second_session,
            0,
            1,
            "0x2222222222222222222222222222222222222222",
            100,
        )
        .expect("second chain metadata");
    let first_cache_key =
        wallet_cache_row_record_key(&first_chain.wallet_chain_uuid, &[0x11; KEY_LEN]);
    let second_cache_key =
        wallet_cache_row_record_key(&second_chain.wallet_chain_uuid, &[0x22; KEY_LEN]);
    db.put_desktop_wallet_vault_record(&first_cache_key, b"first cache")
        .expect("store first cache row");
    db.put_desktop_wallet_vault_record(&second_cache_key, b"second cache")
        .expect("store second cache row");
    let private_account = store
        .import_public_account(
            TEST_PASSWORD,
            &second_session,
            IMPORT_PRIVATE_KEY_ONE,
            Some("Private"),
            false,
        )
        .expect("import private scoped account");
    let global_account = store
        .import_public_account(
            TEST_PASSWORD,
            &second_session,
            IMPORT_PRIVATE_KEY_TWO,
            Some("Global"),
            true,
        )
        .expect("import global account");
    let private_account_ids = store
        .list_public_accounts_for_session(&second_session, true)
        .expect("list second wallet public accounts")
        .into_iter()
        .filter_map(|account| match account.scope {
            PublicAccountScope::PrivateWallet { wallet_uuid }
                if wallet_uuid == second_wallet_id =>
            {
                Some(account.public_account_uuid)
            }
            PublicAccountScope::PrivateWallet { .. } | PublicAccountScope::Global => None,
        })
        .collect::<Vec<_>>();
    assert!(private_account_ids.contains(&private_account.public_account_uuid));

    let deleted = store
        .delete_wallet_for_session(&first_session, second_wallet_id)
        .expect("delete active wallet");
    assert_eq!(deleted.wallet_uuid, second_wallet_id);
    assert_eq!(deleted.status, WalletStatus::Active);
    assert!(
        store
            .load_view_session(TEST_PASSWORD, second_wallet_id)
            .is_err()
    );

    for key in [
        wallet_metadata_record_key(second_wallet_id),
        wallet_view_record_key(second_wallet_id),
        wallet_spend_record_key(second_wallet_id),
        wallet_chain_metadata_record_key(&second_chain.wallet_chain_uuid),
        second_cache_key,
    ] {
        assert!(
            db.get_desktop_wallet_vault_record(&key)
                .expect("load deleted record")
                .is_none(),
            "expected {key} to be deleted"
        );
    }
    for key in [
        wallet_metadata_record_key(first_wallet_id),
        wallet_view_record_key(first_wallet_id),
        wallet_spend_record_key(first_wallet_id),
        wallet_chain_metadata_record_key(&first_chain.wallet_chain_uuid),
        first_cache_key,
    ] {
        assert!(
            db.get_desktop_wallet_vault_record(&key)
                .expect("load retained record")
                .is_some(),
            "expected {key} to be retained"
        );
    }
    for account_id in private_account_ids {
        assert!(
            db.get_desktop_wallet_vault_record(&public_account_metadata_record_key(&account_id))
                .expect("load deleted public account metadata")
                .is_none()
        );
        assert!(
            db.get_desktop_wallet_vault_record(&public_account_secret_record_key(&account_id))
                .expect("load deleted public account secret")
                .is_none()
        );
    }
    assert!(
        db.get_desktop_wallet_vault_record(&public_account_metadata_record_key(
            &global_account.public_account_uuid,
        ))
        .expect("load global metadata")
        .is_some()
    );
    assert!(
        db.get_desktop_wallet_vault_record(&public_account_secret_record_key(
            &global_account.public_account_uuid,
        ))
        .expect("load global secret")
        .is_some()
    );

    store
        .set_wallet_active_for_session(&first_session, third_wallet_id, false)
        .expect("hide third wallet");
    let deleted_hidden = store
        .delete_wallet_for_session(&first_session, third_wallet_id)
        .expect("delete hidden wallet");
    assert_eq!(deleted_hidden.status, WalletStatus::Inactive);
    assert!(
        store
            .load_view_session(TEST_PASSWORD, third_wallet_id)
            .is_err()
    );
    assert!(matches!(
        store.delete_wallet_for_session(&first_session, first_wallet_id),
        Err(VaultError::LastActiveWallet)
    ));
    assert_eq!(
        store
            .list_wallet_metadata_for_session(&first_session, true)
            .expect("list remaining metadata")
            .iter()
            .map(|metadata| metadata.wallet_uuid.as_str())
            .collect::<Vec<_>>(),
        vec![first_wallet_id]
    );

    drop(store);
    drop(db);
    fs::remove_dir_all(root_dir).expect("remove temp db dir");
}
#[test]
fn duplicate_seed_imports_keep_distinct_wallet_and_chain_ids() {
    let (root_dir, db, store) = desktop_store_with_vault();
    let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let first_wallet_id = "duplicate-seed-a";
    let first_metadata = store
        .new_wallet_metadata(
            TEST_PASSWORD,
            first_wallet_id,
            0,
            WalletSource::Imported,
            "Duplicate A",
        )
        .expect("first duplicate metadata");
    store
        .import_wallet_mnemonic_with_metadata(
            TEST_PASSWORD,
            first_wallet_id,
            0,
            "english",
            mnemonic,
            &first_metadata,
        )
        .expect("import first duplicate seed");

    let second_wallet_id = "duplicate-seed-b";
    let second_metadata = store
        .new_wallet_metadata(
            TEST_PASSWORD,
            second_wallet_id,
            0,
            WalletSource::Imported,
            "Duplicate B",
        )
        .expect("second duplicate metadata");
    store
        .import_wallet_mnemonic_with_metadata(
            TEST_PASSWORD,
            second_wallet_id,
            0,
            "english",
            mnemonic,
            &second_metadata,
        )
        .expect("import second duplicate seed");

    let first_session = store
        .load_view_session(TEST_PASSWORD, first_wallet_id)
        .expect("load first session");
    let second_session = store
        .load_view_session(TEST_PASSWORD, second_wallet_id)
        .expect("load second session");
    let first_chain = store
        .wallet_chain_metadata_for_session(
            &first_session,
            0,
            1,
            "0x1111111111111111111111111111111111111111",
            100,
        )
        .expect("first chain metadata");
    let second_chain = store
        .wallet_chain_metadata_for_session(
            &second_session,
            0,
            1,
            "0x1111111111111111111111111111111111111111",
            100,
        )
        .expect("second chain metadata");

    assert_ne!(first_wallet_id, second_wallet_id);
    assert_ne!(
        first_chain.wallet_chain_uuid,
        second_chain.wallet_chain_uuid
    );
    assert_eq!(first_chain.wallet_uuid, first_wallet_id);
    assert_eq!(second_chain.wallet_uuid, second_wallet_id);
    assert_eq!(
        first_session.scan_keys().master_public_key,
        second_session.scan_keys().master_public_key
    );
    assert_eq!(
        first_session.scan_keys().nullifying_key,
        second_session.scan_keys().nullifying_key
    );

    drop(store);
    drop(db);
    fs::remove_dir_all(root_dir).expect("remove temp db dir");
}
#[test]
fn opaque_wallet_metadata_keeps_chain_details_encrypted() {
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
    let wallet_uuid = generate_opaque_id().expect("wallet uuid");
    let wallet_chain_uuid = generate_opaque_id().expect("wallet chain uuid");
    let wallet_metadata = WalletMetadataBundle {
        wallet_uuid: wallet_uuid.clone(),
        label: "primary wallet".to_string(),
        derivation_index: 0,
        source: WalletSource::Imported,
        status: WalletStatus::Active,
        display_order: 0,
        hardware_descriptor: None,
        hardware_account: None,
    };
    let chain_metadata = WalletChainMetadataBundle {
        wallet_chain_uuid: wallet_chain_uuid.clone(),
        wallet_uuid: wallet_uuid.clone(),
        chain_type: 0,
        chain_id: 1,
        contract: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
        start_block: 100,
        last_scanned_block: 200,
        last_scanned_block_hash: Some([9u8; KEY_LEN]),
        poi_read_source: None,
    };

    store
        .store_wallet_metadata(TEST_PASSWORD, &wallet_metadata)
        .expect("store wallet metadata");
    store
        .store_wallet_chain_metadata(TEST_PASSWORD, &chain_metadata)
        .expect("store chain metadata");
    let wallet_payload = db
        .get_desktop_wallet_vault_record(&wallet_metadata_record_key(&wallet_uuid))
        .expect("load wallet metadata record")
        .expect("wallet metadata present");
    let chain_payload = db
        .get_desktop_wallet_vault_record(&wallet_chain_metadata_record_key(&wallet_chain_uuid))
        .expect("load chain metadata record")
        .expect("chain metadata present");
    let loaded_wallet = store
        .load_wallet_metadata(TEST_PASSWORD, &wallet_uuid)
        .expect("load wallet metadata");
    let loaded_chain = store
        .load_wallet_chain_metadata(TEST_PASSWORD, &wallet_chain_uuid)
        .expect("load chain metadata");

    assert_eq!(wallet_uuid.len(), 32);
    assert_eq!(wallet_chain_uuid.len(), 32);
    assert_eq!(loaded_wallet.label, "primary wallet");
    assert_eq!(loaded_chain.chain_id, 1);
    assert_eq!(loaded_chain.contract, chain_metadata.contract);
    assert!(!contains_subsequence(&wallet_payload, b"primary wallet"));
    assert!(!contains_subsequence(&chain_payload, b"1234567890abcdef"));

    drop(store);
    drop(db);
    fs::remove_dir_all(root_dir).expect("remove temp db dir");
}
