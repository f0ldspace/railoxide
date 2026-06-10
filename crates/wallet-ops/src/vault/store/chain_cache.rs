use super::*;

impl DesktopVaultStore {
    pub fn store_wallet_chain_metadata(
        &self,
        password: &str,
        metadata: &WalletChainMetadataBundle,
    ) -> Result<(), VaultError> {
        let view = self.unlock_view(password)?;
        let record = view.encrypt_wallet_chain_metadata(&metadata.wallet_chain_uuid, metadata)?;
        self.put_encrypted_record(
            &wallet_chain_metadata_record_key(&metadata.wallet_chain_uuid),
            &record,
        )
    }

    pub fn load_wallet_chain_metadata(
        &self,
        password: &str,
        wallet_chain_uuid: &str,
    ) -> Result<WalletChainMetadataBundle, VaultError> {
        let view = self.unlock_view(password)?;
        let record = self.encrypted_record(&wallet_chain_metadata_record_key(wallet_chain_uuid))?;
        view.decrypt_wallet_chain_metadata(wallet_chain_uuid, &record)
    }

    pub fn wallet_chain_metadata_for_session(
        &self,
        view_session: &DesktopViewSession,
        chain_type: u8,
        chain_id: u64,
        contract: &str,
        start_block: u64,
    ) -> Result<WalletChainMetadataBundle, VaultError> {
        if let Some(metadata) = self.find_wallet_chain_metadata_for_session(
            view_session,
            chain_type,
            chain_id,
            contract,
        )? {
            return Ok(metadata);
        }

        self.create_wallet_chain_metadata_for_session(
            view_session,
            chain_type,
            chain_id,
            contract,
            start_block,
            start_block.saturating_sub(1),
        )
    }

    pub fn find_wallet_chain_metadata_for_session(
        &self,
        view_session: &DesktopViewSession,
        chain_type: u8,
        chain_id: u64,
        contract: &str,
    ) -> Result<Option<WalletChainMetadataBundle>, VaultError> {
        let records = self
            .db
            .list_desktop_wallet_vault_records(WALLET_CHAIN_METADATA_PREFIX)?;
        for stored in records {
            let Some(wallet_chain_uuid) = stored.key.strip_prefix(WALLET_CHAIN_METADATA_PREFIX)
            else {
                continue;
            };
            let Ok(record) = rmp_serde::from_slice::<EncryptedRecord>(&stored.payload) else {
                tracing::warn!("ignoring invalid wallet chain metadata record during lookup");
                continue;
            };
            let Ok(metadata) =
                view_session.decrypt_wallet_chain_metadata(wallet_chain_uuid, &record)
            else {
                continue;
            };
            if metadata.wallet_uuid == view_session.wallet_id()
                && metadata.chain_type == chain_type
                && metadata.chain_id == chain_id
                && metadata.contract.eq_ignore_ascii_case(contract)
            {
                return Ok(Some(metadata));
            }
        }

        Ok(None)
    }

    pub fn create_wallet_chain_metadata_for_session(
        &self,
        view_session: &DesktopViewSession,
        chain_type: u8,
        chain_id: u64,
        contract: &str,
        start_block: u64,
        last_scanned_block: u64,
    ) -> Result<WalletChainMetadataBundle, VaultError> {
        let wallet_chain_uuid = generate_opaque_id()?;
        let metadata = WalletChainMetadataBundle {
            wallet_chain_uuid,
            wallet_uuid: view_session.wallet_id().to_owned(),
            chain_type,
            chain_id,
            contract: contract.to_owned(),
            start_block,
            last_scanned_block,
            last_scanned_block_hash: None,
            poi_read_source: None,
        };
        self.store_wallet_chain_metadata_with_session(view_session, &metadata)?;
        Ok(metadata)
    }

    pub fn store_wallet_chain_metadata_with_session(
        &self,
        view_session: &DesktopViewSession,
        metadata: &WalletChainMetadataBundle,
    ) -> Result<(), VaultError> {
        let record =
            view_session.encrypt_wallet_chain_metadata(&metadata.wallet_chain_uuid, metadata)?;
        self.put_encrypted_record(
            &wallet_chain_metadata_record_key(&metadata.wallet_chain_uuid),
            &record,
        )
    }

    pub fn reset_wallet_chain_cache_with_session(
        &self,
        view_session: &DesktopViewSession,
        metadata: &mut WalletChainMetadataBundle,
        start_block: u64,
    ) -> Result<(), VaultError> {
        metadata.last_scanned_block = start_block.saturating_sub(1);
        metadata.last_scanned_block_hash = None;
        let record =
            view_session.encrypt_wallet_chain_metadata(&metadata.wallet_chain_uuid, metadata)?;
        let data = rmp_serde::to_vec_named(&record)?;
        self.db.replace_desktop_wallet_vault_prefix_with_records(
            &wallet_cache_row_prefix(&metadata.wallet_chain_uuid),
            &[(
                wallet_chain_metadata_record_key(&metadata.wallet_chain_uuid),
                data,
            )],
        )?;
        Ok(())
    }

    pub fn rewind_wallet_chain_cache_with_session(
        &self,
        view_session: &DesktopViewSession,
        metadata: &mut WalletChainMetadataBundle,
        start_block: u64,
    ) -> Result<(), VaultError> {
        let cache_keys = view_session.derive_cache_keys(&metadata.wallet_chain_uuid)?;
        let row_prefix = wallet_cache_row_prefix(&metadata.wallet_chain_uuid);
        let existing_rows = self.db.list_desktop_wallet_vault_records(&row_prefix)?;
        let mut records = Vec::with_capacity(existing_rows.len() + 1);
        let mut dropped_rows = 0usize;
        let mut cleared_spent_rows = 0usize;
        let mut invalid_rows = 0usize;

        for stored in existing_rows {
            let Some(row_id_hex) = stored.key.strip_prefix(&row_prefix) else {
                invalid_rows += 1;
                continue;
            };
            let Ok(row_id_bytes) = alloy::hex::decode(row_id_hex) else {
                invalid_rows += 1;
                continue;
            };
            let Ok(row_id) = row_id_bytes.try_into() else {
                invalid_rows += 1;
                continue;
            };
            let record: EncryptedRecord = rmp_serde::from_slice(&stored.payload)?;
            let plaintext = cache_keys
                .decrypt_row(&row_id, &record)
                .map_err(|_| VaultError::Decrypt)?;
            let mut utxo =
                deserialize_wallet_utxo(&plaintext).map_err(vault_error_from_wallet_cache)?;
            if utxo.utxo.source.block_number >= start_block {
                dropped_rows += 1;
                continue;
            }
            if utxo
                .spent
                .as_ref()
                .is_some_and(|spent| spent.block_number >= start_block)
            {
                utxo.spent = None;
                cleared_spent_rows += 1;
            }

            let stable_identity = wallet_utxo_stable_identity(&utxo);
            let expected_row_id =
                cache_keys.row_id(utxo.utxo.tree, utxo.utxo.position, &stable_identity);
            if expected_row_id != row_id {
                invalid_rows += 1;
                continue;
            }

            let plaintext = serialize_wallet_utxo(&utxo).map_err(vault_error_from_wallet_cache)?;
            let record = cache_keys
                .encrypt_row(&row_id, &plaintext)
                .map_err(|_| VaultError::Encrypt)?;
            let data = rmp_serde::to_vec_named(&record)?;
            records.push((
                wallet_cache_row_record_key(&metadata.wallet_chain_uuid, &row_id),
                data,
            ));
        }

        metadata.last_scanned_block = start_block.saturating_sub(1);
        metadata.last_scanned_block_hash = None;
        let record =
            view_session.encrypt_wallet_chain_metadata(&metadata.wallet_chain_uuid, metadata)?;
        let data = rmp_serde::to_vec_named(&record)?;
        records.push((
            wallet_chain_metadata_record_key(&metadata.wallet_chain_uuid),
            data,
        ));
        self.db
            .replace_desktop_wallet_vault_prefix_with_records(&row_prefix, &records)?;
        tracing::info!(
            wallet_chain_uuid = %metadata.wallet_chain_uuid,
            start_block,
            retained_rows = records.len().saturating_sub(1),
            dropped_rows,
            cleared_spent_rows,
            invalid_rows,
            "rewound encrypted desktop wallet cache"
        );
        Ok(())
    }
}
