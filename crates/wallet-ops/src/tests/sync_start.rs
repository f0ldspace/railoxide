use super::helpers::*;

#[test]
fn desktop_wallet_start_policy_generated_defaults_to_historical_backfill() {
    let resolved = resolve_desktop_wallet_chain_start(
        DesktopWalletSyncStartPolicy::from(vault::WalletSource::Generated),
        None,
        None,
        100,
        Some(250),
        false,
    )
    .expect("resolve generated start");

    assert_eq!(
        resolved,
        DesktopWalletChainStart {
            start_block: 100,
            last_scanned_block: 99,
        }
    );
}

#[test]
fn desktop_wallet_start_policy_imported_uses_deployment_block() {
    let resolved = resolve_desktop_wallet_chain_start(
        DesktopWalletSyncStartPolicy::ImportedHistoricalBackfill,
        None,
        None,
        100,
        Some(250),
        false,
    )
    .expect("resolve imported start");

    assert_eq!(
        resolved,
        DesktopWalletChainStart {
            start_block: 100,
            last_scanned_block: 99,
        }
    );
}

#[test]
fn desktop_wallet_start_policy_new_hardware_defaults_to_historical_backfill() {
    let metadata = hardware_wallet_metadata(HardwareWalletSyncIntent::CreateNew);
    assert_eq!(
        DesktopWalletSyncStartPolicy::from(&metadata),
        DesktopWalletSyncStartPolicy::ImportedHistoricalBackfill
    );

    let resolved = resolve_desktop_wallet_chain_start(
        DesktopWalletSyncStartPolicy::from(&metadata),
        None,
        None,
        100,
        Some(250),
        false,
    )
    .expect("resolve new hardware start");

    assert_eq!(
        resolved,
        DesktopWalletChainStart {
            start_block: 100,
            last_scanned_block: 99,
        }
    );
}

#[test]
fn desktop_wallet_creation_override_uses_safe_head_no_backfill() {
    let resolved = resolve_desktop_wallet_chain_start(
        DesktopWalletSyncStartPolicy::CurrentSafeHeadNoBackfill,
        None,
        None,
        100,
        Some(250),
        false,
    )
    .expect("resolve generated creation start");

    assert_eq!(
        resolved,
        DesktopWalletChainStart {
            start_block: 251,
            last_scanned_block: 250,
        }
    );
}

#[test]
fn new_wallet_chain_start_helpers_use_expected_baselines() {
    assert_eq!(
        new_wallet_chain_start_from_deployment(100),
        DesktopWalletChainStart {
            start_block: 100,
            last_scanned_block: 99,
        }
    );
    assert_eq!(
        new_wallet_chain_start_from_head(100, 10, 250),
        DesktopWalletChainStart {
            start_block: 241,
            last_scanned_block: 240,
        }
    );
    assert_eq!(
        new_wallet_chain_start_from_head(100, 10, 50),
        DesktopWalletChainStart {
            start_block: 101,
            last_scanned_block: 100,
        }
    );
}

#[test]
fn new_wallet_chain_metadata_initializer_creates_deployment_fallback_metadata() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime");
    let root_dir = temp_db_root();
    let db = Arc::new(
        DbStore::open(DbConfig {
            root_dir: root_dir.clone(),
        })
        .expect("open db"),
    );
    let store = vault::DesktopVaultStore::from_db(Arc::clone(&db));
    store
        .create_vault_with_params(TEST_PASSWORD, vault::KdfParams::new(1024, 1, 1))
        .expect("create vault");
    let wallet_id = "generated-wallet";
    let metadata = store
        .new_wallet_metadata(
            TEST_PASSWORD,
            wallet_id,
            0,
            vault::WalletSource::Generated,
            "Generated",
        )
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
        .expect("store wallet");
    let session = Arc::new(
        store
            .load_view_session(TEST_PASSWORD, wallet_id)
            .expect("load view session"),
    );
    let http = runtime
        .block_on(crate::build_wallet_network_context(
            crate::WalletNetworkConfig {
                network_mode: Some(crate::WalletNetworkMode::Direct),
                proxy: None,
                data_dir: &root_dir,
            },
        ))
        .expect("direct HTTP context");
    let deployment_block = 12_345;
    let configs = BTreeMap::from([(
        1,
        effective_chain_config_with_rpc_endpoints(1, Vec::new(), deployment_block),
    )]);

    let report = runtime.block_on(initialize_new_wallet_chain_metadata_for_session(
        Arc::clone(&session),
        configs.clone(),
        Arc::clone(&db),
        http.clone(),
        None,
    ));

    assert_eq!(report.initialized, 1);
    assert_eq!(report.deployment_fallbacks, 1);
    assert_eq!(report.failed, 0);

    let contract = ChainConfigDefaults::for_chain(1)
        .expect("ethereum defaults")
        .contract
        .to_checksum(None);
    let chain_metadata = store
        .find_wallet_chain_metadata_for_session(session.as_ref(), 0, 1, &contract)
        .expect("load chain metadata")
        .expect("chain metadata exists");
    assert_eq!(chain_metadata.start_block, deployment_block);
    assert_eq!(
        chain_metadata.last_scanned_block,
        deployment_block.saturating_sub(1)
    );

    let report = runtime.block_on(initialize_new_wallet_chain_metadata_for_session(
        session, configs, db, http, None,
    ));
    assert_eq!(report.initialized, 0);
    assert_eq!(report.skipped_existing, 1);

    drop(store);
    let _ = fs::remove_dir_all(root_dir);
}

#[test]
fn desktop_wallet_start_policy_recovered_hardware_uses_deployment_block() {
    let metadata = hardware_wallet_metadata(HardwareWalletSyncIntent::RecoverExisting);
    assert_eq!(
        DesktopWalletSyncStartPolicy::from(&metadata),
        DesktopWalletSyncStartPolicy::ImportedHistoricalBackfill
    );

    let resolved = resolve_desktop_wallet_chain_start(
        DesktopWalletSyncStartPolicy::from(&metadata),
        None,
        None,
        100,
        Some(250),
        false,
    )
    .expect("resolve recovered hardware start");

    assert_eq!(
        resolved,
        DesktopWalletChainStart {
            start_block: 100,
            last_scanned_block: 99,
        }
    );
}

#[test]
fn chain_config_uses_effective_rpc_pool_and_sync_tuning() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime");
    let root_dir = temp_db_root();
    let http = runtime
        .block_on(crate::build_wallet_network_context(
            crate::WalletNetworkConfig {
                network_mode: Some(crate::WalletNetworkMode::Direct),
                proxy: None,
                data_dir: &root_dir,
            },
        ))
        .expect("direct HTTP context");
    let defaults = ChainConfigDefaults::for_chain(1).expect("ethereum defaults");
    let effective = crate::settings::EffectiveChainConfig {
        chain_id: 1,
        enabled: true,
        rpc_endpoints: vec![
            "https://rpc-a.example".to_string(),
            "https://rpc-b.example".to_string(),
        ],
        archive_rpc_url: Some("https://archive.example".to_string()),
        quick_sync_enabled: false,
        quick_sync_endpoint: Some("https://quick.example/graphql".to_string()),
        indexed_wallet_block_range: 12_345,
        deployment_block: 12_000,
        v2_start_block: 13_000,
        legacy_shield_block: 14_000,
        archive_until_block: 12_500,
        railgun_contract: defaults.contract.to_string(),
        relay_adapt_contract: defaults.relay_adapt_contract.to_string(),
        relay_adapt_7702_contract: defaults.relay_adapt_7702_contract.to_string(),
        wrapped_native_token: wrapped_native_token_for_chain(1).map(|token| token.to_string()),
        multicall_contract: defaults.multicall_contract.to_string(),
        finality_depth: 99,
        block_range: Some(2_000),
        poll_interval_secs: Some(30),
        gas: crate::settings::EffectiveChainGasSettings {
            gas_limit_buffer: 250_000,
            gas_price_buffer_numerator: 110,
            gas_price_buffer_denominator: 100,
        },
    };

    let cfg = crate::chain_config(
        &defaults,
        Some(reqwest::Url::parse("https://ignored.example").expect("url")),
        Some(&effective),
        &http,
        None,
    )
    .expect("chain config");

    assert_eq!(cfg.quick_sync_endpoint, None);
    assert_eq!(cfg.indexed_wallet_block_range, 12_345);
    assert_eq!(cfg.finality_depth, 99);
    assert_eq!(cfg.block_range, 2_000);
    assert_eq!(cfg.poll_interval, Duration::from_secs(30));
    assert_eq!(
        cfg.archive_rpc_url.as_ref().map(reqwest::Url::as_str),
        Some("https://archive.example/")
    );
    assert_eq!(cfg.deployment_block, 12_000);
    assert_eq!(cfg.v2_start_block, 13_000);
    assert_eq!(cfg.legacy_shield_block, 14_000);
    assert_eq!(cfg.archive_until_block, 12_500);

    let first = cfg.rpcs.random_provider().expect("first provider");
    cfg.rpcs.mark_bad_provider(&first);
    let second = cfg.rpcs.random_provider().expect("fallback provider");
    assert_ne!(first.url, second.url);

    drop(http);
    let _ = fs::remove_dir_all(root_dir);
}

#[test]
fn desktop_wallet_start_policy_reuses_existing_metadata() {
    let existing = crate::vault::WalletChainMetadataBundle {
        wallet_chain_uuid: "wallet-chain".to_string(),
        wallet_uuid: "wallet".to_string(),
        chain_type: 0,
        chain_id: 1,
        contract: "0x1111111111111111111111111111111111111111".to_string(),
        start_block: 251,
        last_scanned_block: 300,
        last_scanned_block_hash: None,
        poi_read_source: None,
    };

    let resolved = resolve_desktop_wallet_chain_start(
        DesktopWalletSyncStartPolicy::CurrentSafeHeadNoBackfill,
        Some(&existing),
        None,
        100,
        None,
        false,
    )
    .expect("resolve existing start");

    assert_eq!(
        resolved,
        DesktopWalletChainStart {
            start_block: 251,
            last_scanned_block: 300,
        }
    );
}

#[test]
fn desktop_wallet_start_policy_generated_requires_safe_head() {
    let error = resolve_desktop_wallet_chain_start(
        DesktopWalletSyncStartPolicy::CurrentSafeHeadNoBackfill,
        None,
        None,
        100,
        None,
        false,
    )
    .expect_err("safe head required");

    assert!(error.to_string().contains("safe head unavailable"));
}

#[test]
fn desktop_wallet_rewind_uses_explicit_init_block() {
    let existing = crate::vault::WalletChainMetadataBundle {
        wallet_chain_uuid: "wallet-chain".to_string(),
        wallet_uuid: "wallet".to_string(),
        chain_type: 0,
        chain_id: 1,
        contract: "0x1111111111111111111111111111111111111111".to_string(),
        start_block: 251,
        last_scanned_block: 300,
        last_scanned_block_hash: None,
        poi_read_source: None,
    };

    let resolved = resolve_desktop_wallet_chain_start(
        DesktopWalletSyncStartPolicy::CurrentSafeHeadNoBackfill,
        Some(&existing),
        Some(existing.start_block),
        100,
        None,
        true,
    )
    .expect("resolve explicit rewind start");

    assert_eq!(
        resolved,
        DesktopWalletChainStart {
            start_block: existing.start_block,
            last_scanned_block: existing.start_block.saturating_sub(1),
        }
    );
}
