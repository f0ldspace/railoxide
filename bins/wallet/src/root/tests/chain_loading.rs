use super::*;

#[test]
fn chain_load_uses_default_sync_options() {
    let overrides = super::chain_load_overrides();

    assert_eq!(overrides.init_block_number, None);
    assert_eq!(overrides.sync_to_block, None);
    assert_eq!(overrides.sync_start_policy, None);
    assert!(overrides.use_indexed_wallet_catch_up);
    assert!(!overrides.rewind_wallet_cache);
}

#[test]
fn repair_cache_block_parses_zero_as_deployment() {
    assert_eq!(parse_repair_cache_block("0"), Ok(None));
    assert_eq!(parse_repair_cache_block(""), Ok(None));
    assert_eq!(parse_repair_cache_block(" 24936249 "), Ok(Some(24936249)));
    assert!(parse_repair_cache_block("nope").is_err());
}

#[test]
fn repair_cache_help_text_only_mentions_hint_when_available() {
    assert!(repair_cache_help_text(true).contains("wallet start block below"));
    assert!(!repair_cache_help_text(false).contains("wallet start block below"));
    assert!(repair_cache_help_text(false).contains("deployment block"));
}

#[test]
fn chain_error_state_preserves_start_block_hint() {
    let state = ChainUtxoState::Error {
        message: Arc::from("sync failed"),
        start_block: Some(24936250),
    };

    assert_eq!(state.start_block(), Some(24936250));
    assert!(!state.renders_table());
}

#[test]
fn loading_summary_uses_sync_stage_and_percent() {
    let commitments =
        SyncProgressUpdate::new(SyncProgressStage::SynchronizingCommitments, 100, 150, 300);
    let preparing =
        SyncProgressUpdate::artifact_chunk(SyncProgressStage::PreparingUtxoIndex, 25, 100, 3, 12);
    let indexing = SyncProgressUpdate::new(SyncProgressStage::IndexingUtxos, 100, 150, 300);

    assert_eq!(
        loading_summary(Some(commitments)),
        "Synchronizing commitments · 25%"
    );
    assert_eq!(
        loading_summary(Some(preparing)),
        "Preparing UTXO index · 25%"
    );
    assert_eq!(loading_summary(Some(indexing)), "Indexing UTXOs · 25%");
    assert_eq!(loading_summary(None), "Preparing wallet sync...");
}

#[test]
fn sync_status_labels_describe_no_progress_context() {
    assert_eq!(
        sync_status_labels(SyncStatusContext::Loading, None),
        SyncStatusLabels {
            title: "Preparing wallet sync".to_string(),
            percent: 0,
            detail: "Connecting to chain and loading local wallet state...".to_string(),
        }
    );
    assert_eq!(
        sync_status_labels(SyncStatusContext::Syncing, None),
        SyncStatusLabels {
            title: "Checking wallet sync".to_string(),
            percent: 0,
            detail: "Checking for new wallet events...".to_string(),
        }
    );
}

#[test]
fn sync_status_labels_use_progress_when_available() {
    let progress = SyncProgressUpdate::new(SyncProgressStage::IndexingUtxos, 100, 150, 300);

    assert_eq!(
        sync_status_labels(SyncStatusContext::Loading, Some(progress)),
        SyncStatusLabels {
            title: "Indexing UTXOs".to_string(),
            percent: 25,
            detail: "Block 150 of 300".to_string(),
        }
    );
}

#[test]
fn loading_chain_state_keeps_utxo_table_available() {
    let state = ChainUtxoState::Loading { progress: None };

    assert!(state.renders_table());
    assert!(state.is_syncing());
    assert!(!matches!(state, ChainUtxoState::Ready { .. }));
    assert!(state.snapshot().is_none());
}

#[test]
fn progress_detail_clamps_current_block() {
    let progress = SyncProgressUpdate::new(SyncProgressStage::IndexingUtxos, 100, 400, 300);

    assert_eq!(progress_detail(progress), "Block 300 of 300");
}

#[test]
fn progress_detail_uses_artifact_chunks_for_utxo_prep() {
    let progress =
        SyncProgressUpdate::artifact_chunk(SyncProgressStage::PreparingUtxoIndex, 58, 100, 7, 12);

    assert_eq!(progress_detail(progress), "Artifact chunk 7 of 12");
}

#[test]
fn progress_detail_describes_pending_artifact_chunks() {
    let progress =
        SyncProgressUpdate::artifact_chunk(SyncProgressStage::PreparingUtxoIndex, 25, 100, 0, 11);

    assert_eq!(
        progress_detail(progress),
        "Downloading 11 artifact chunks..."
    );
}

#[test]
fn progress_detail_describes_artifact_metadata() {
    let progress =
        SyncProgressUpdate::artifact_preparation(SyncProgressStage::PreparingUtxoIndex, 5, 100);

    assert_eq!(progress_detail(progress), "Preparing artifact metadata...");
}

#[test]
fn progress_detail_describes_artifact_apply_completion() {
    let progress =
        SyncProgressUpdate::artifact_applied(SyncProgressStage::SynchronizingCommitments);

    assert_eq!(progress_detail(progress), "Commitment artifacts applied");
}

#[test]
fn progress_detail_describes_commitment_tail() {
    let progress = SyncProgressUpdate::commitment_tail(200, 225, 300);

    assert_eq!(
        progress_detail(progress),
        "Checking commitment tail: block 225 of 300"
    );
}
