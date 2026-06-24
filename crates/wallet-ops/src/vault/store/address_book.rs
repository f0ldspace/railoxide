use super::{
    DesktopVaultStore, DesktopViewSession, EncryptedRecord, PRIVATE_ADDRESS_BOOK_PREFIX,
    PUBLIC_ADDRESS_BOOK_PREFIX, PrivateAddressBookEntry, PublicAddressBookEntry, VaultError,
    ViewUnlock, WalletStatus, ensure_private_address_book_address_available,
    ensure_private_address_book_address_available_for_update,
    ensure_public_address_book_address_available,
    ensure_public_address_book_address_available_for_update, generate_opaque_id,
    next_private_address_book_display_order, next_public_address_book_display_order,
    private_address_book_record_entry, private_address_book_record_key,
    public_address_book_record_entry, public_address_book_record_key,
    sort_private_address_book_entries, sort_public_address_book_entries,
    validate_address_book_label, validate_private_address_book_address,
    validate_public_address_book_address,
};

impl DesktopVaultStore {
    pub fn list_private_address_book_entries_for_session(
        &self,
        view_session: &DesktopViewSession,
    ) -> Result<Vec<PrivateAddressBookEntry>, VaultError> {
        self.list_private_address_book_entries_with_view(&view_session.view)
    }

    pub fn list_public_address_book_entries_for_session(
        &self,
        view_session: &DesktopViewSession,
    ) -> Result<Vec<PublicAddressBookEntry>, VaultError> {
        self.list_public_address_book_entries_with_view(&view_session.view)
    }

    pub fn add_private_address_book_entry_for_session(
        &self,
        view_session: &DesktopViewSession,
        label: &str,
        address: &str,
    ) -> Result<PrivateAddressBookEntry, VaultError> {
        let label = validate_address_book_label(label)?;
        let address = validate_private_address_book_address(address)?;
        let entries = self.list_private_address_book_entries_with_view(&view_session.view)?;
        let active_private_recipients = self.active_private_receive_addresses(view_session)?;
        ensure_private_address_book_address_available(
            &entries,
            &active_private_recipients,
            &address,
        )?;

        let entry = PrivateAddressBookEntry {
            entry_uuid: generate_opaque_id()?,
            label,
            address,
            display_order: next_private_address_book_display_order(&entries)?,
        };
        let (key, data) = private_address_book_record_entry(&view_session.view, &entry)?;
        self.db.put_desktop_wallet_vault_record(&key, &data)?;
        Ok(entry)
    }

    pub fn add_public_address_book_entry_for_session(
        &self,
        view_session: &DesktopViewSession,
        label: &str,
        address: &str,
    ) -> Result<PublicAddressBookEntry, VaultError> {
        let label = validate_address_book_label(label)?;
        let address = validate_public_address_book_address(address)?;
        let entries = self.list_public_address_book_entries_with_view(&view_session.view)?;
        let accounts = self.list_public_accounts_for_session(view_session, false)?;
        ensure_public_address_book_address_available(&entries, &accounts, address)?;

        let entry = PublicAddressBookEntry {
            entry_uuid: generate_opaque_id()?,
            label,
            address,
            display_order: next_public_address_book_display_order(&entries)?,
        };
        let (key, data) = public_address_book_record_entry(&view_session.view, &entry)?;
        self.db.put_desktop_wallet_vault_record(&key, &data)?;
        Ok(entry)
    }

    pub fn update_private_address_book_entry_for_session(
        &self,
        view_session: &DesktopViewSession,
        entry_uuid: &str,
        label: &str,
        address: &str,
    ) -> Result<PrivateAddressBookEntry, VaultError> {
        let label = validate_address_book_label(label)?;
        let address = validate_private_address_book_address(address)?;
        let mut entries = self.list_private_address_book_entries_with_view(&view_session.view)?;
        let Some(entry_index) = entries
            .iter()
            .position(|entry| entry.entry_uuid == entry_uuid)
        else {
            return Err(VaultError::PrivateAddressBookEntryNotFound);
        };
        let active_private_recipients = self.active_private_receive_addresses(view_session)?;
        ensure_private_address_book_address_available_for_update(
            &entries,
            &active_private_recipients,
            entry_uuid,
            &address,
        )?;

        entries[entry_index].label = label;
        entries[entry_index].address = address;
        let updated = entries[entry_index].clone();
        let (key, data) = private_address_book_record_entry(&view_session.view, &updated)?;
        self.db.put_desktop_wallet_vault_record(&key, &data)?;
        Ok(updated)
    }

    pub fn update_public_address_book_entry_for_session(
        &self,
        view_session: &DesktopViewSession,
        entry_uuid: &str,
        label: &str,
        address: &str,
    ) -> Result<PublicAddressBookEntry, VaultError> {
        let label = validate_address_book_label(label)?;
        let address = validate_public_address_book_address(address)?;
        let mut entries = self.list_public_address_book_entries_with_view(&view_session.view)?;
        let Some(entry_index) = entries
            .iter()
            .position(|entry| entry.entry_uuid == entry_uuid)
        else {
            return Err(VaultError::PublicAddressBookEntryNotFound);
        };
        let accounts = self.list_public_accounts_for_session(view_session, false)?;
        ensure_public_address_book_address_available_for_update(
            &entries, &accounts, entry_uuid, address,
        )?;

        entries[entry_index].label = label;
        entries[entry_index].address = address;
        let updated = entries[entry_index].clone();
        let (key, data) = public_address_book_record_entry(&view_session.view, &updated)?;
        self.db.put_desktop_wallet_vault_record(&key, &data)?;
        Ok(updated)
    }

    pub fn delete_private_address_book_entry_for_session(
        &self,
        view_session: &DesktopViewSession,
        entry_uuid: &str,
    ) -> Result<PrivateAddressBookEntry, VaultError> {
        let entries = self.list_private_address_book_entries_with_view(&view_session.view)?;
        let Some(entry) = entries
            .into_iter()
            .find(|entry| entry.entry_uuid == entry_uuid)
        else {
            return Err(VaultError::PrivateAddressBookEntryNotFound);
        };
        self.db
            .delete_desktop_wallet_vault_record(&private_address_book_record_key(entry_uuid))?;
        Ok(entry)
    }

    pub fn delete_public_address_book_entry_for_session(
        &self,
        view_session: &DesktopViewSession,
        entry_uuid: &str,
    ) -> Result<PublicAddressBookEntry, VaultError> {
        let entries = self.list_public_address_book_entries_with_view(&view_session.view)?;
        let Some(entry) = entries
            .into_iter()
            .find(|entry| entry.entry_uuid == entry_uuid)
        else {
            return Err(VaultError::PublicAddressBookEntryNotFound);
        };
        self.db
            .delete_desktop_wallet_vault_record(&public_address_book_record_key(entry_uuid))?;
        Ok(entry)
    }

    fn active_private_receive_addresses(
        &self,
        view_session: &DesktopViewSession,
    ) -> Result<Vec<String>, VaultError> {
        let mut metadata = self.list_wallet_metadata_with_view(&view_session.view)?;
        metadata.retain(|metadata| metadata.status == WalletStatus::Active);
        let mut addresses = Vec::with_capacity(metadata.len());
        if let Ok(address) = view_session.receive_address() {
            addresses.push(address);
        }
        for metadata in metadata {
            if metadata.wallet_uuid == view_session.wallet_id() {
                continue;
            }
            if let Some(address) = metadata
                .hardware_account
                .as_ref()
                .and_then(|account| account.receive_address.as_ref())
            {
                addresses.push(address.clone());
                continue;
            }
            if metadata.source.is_hardware_derived() {
                continue;
            }
            let session =
                self.load_view_session_with_view_session(view_session, &metadata.wallet_uuid)?;
            addresses.push(
                session
                    .receive_address()
                    .map_err(|_| VaultError::InvalidPrivateAddressBookAddress)?,
            );
        }
        Ok(addresses)
    }

    fn list_private_address_book_entries_with_view(
        &self,
        view: &ViewUnlock,
    ) -> Result<Vec<PrivateAddressBookEntry>, VaultError> {
        let records = self
            .db
            .list_desktop_wallet_vault_records(PRIVATE_ADDRESS_BOOK_PREFIX)?;
        let mut entries = Vec::with_capacity(records.len());
        for stored in records {
            let Some(entry_uuid) = stored.key.strip_prefix(PRIVATE_ADDRESS_BOOK_PREFIX) else {
                continue;
            };
            let record: EncryptedRecord = rmp_serde::from_slice(&stored.payload)?;
            let mut entry = view.decrypt_private_address_book_entry(entry_uuid, &record)?;
            if entry.entry_uuid != entry_uuid {
                entry_uuid.clone_into(&mut entry.entry_uuid);
            }
            entries.push(entry);
        }
        sort_private_address_book_entries(&mut entries);
        Ok(entries)
    }

    fn list_public_address_book_entries_with_view(
        &self,
        view: &ViewUnlock,
    ) -> Result<Vec<PublicAddressBookEntry>, VaultError> {
        let records = self
            .db
            .list_desktop_wallet_vault_records(PUBLIC_ADDRESS_BOOK_PREFIX)?;
        let mut entries = Vec::with_capacity(records.len());
        for stored in records {
            let Some(entry_uuid) = stored.key.strip_prefix(PUBLIC_ADDRESS_BOOK_PREFIX) else {
                continue;
            };
            let record: EncryptedRecord = rmp_serde::from_slice(&stored.payload)?;
            let mut entry = view.decrypt_public_address_book_entry(entry_uuid, &record)?;
            if entry.entry_uuid != entry_uuid {
                entry_uuid.clone_into(&mut entry.entry_uuid);
            }
            entries.push(entry);
        }
        sort_public_address_book_entries(&mut entries);
        Ok(entries)
    }
}
