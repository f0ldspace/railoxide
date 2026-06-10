use alloy::primitives::Address;
use gpui::{ParentElement, Styled, div, px, rgb};
use gpui_component::{IconName, Sizable, spinner::Spinner};
use ui::controls::{app_muted_text, app_strong_text};
use ui::theme::{self, APP_MONO_FONT_FAMILY, APP_TEXT_SIZE};
use wallet_ops::hardware::HardwareDeviceKind;

use crate::root::ui_helpers::rgb_with_alpha;

#[cfg(feature = "hardware")]
use std::sync::Arc;

#[cfg(feature = "hardware")]
use wallet_ops::{
    hardware::trezor::TrezorPinMatrixProvider,
    hardware::{
        DEFAULT_HARDWARE_DERIVATION_PATH, HardwarePublicAccountDescriptor, parse_bip32_path,
    },
    vault::{DesktopVaultStore, DesktopViewSession, HardwareProfileSession, PublicAccountMetadata},
};
#[cfg(feature = "hardware")]
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::root) enum HardwarePublicAccountDerivationStatus {
    Idle,
    CheckingDevice,
    AwaitingAddressConfirmation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg(feature = "hardware")]
pub(super) enum HardwarePublicAccountDerivationProgress {
    CheckingDevice,
    AwaitingAddressConfirmation(Address),
}

pub(super) const fn hardware_public_device_label(device_kind: HardwareDeviceKind) -> &'static str {
    match device_kind {
        HardwareDeviceKind::Ledger => "Ledger",
        HardwareDeviceKind::Trezor => "Trezor",
    }
}

pub(super) fn render_hardware_public_account_checking(
    device_kind: HardwareDeviceKind,
) -> gpui::Div {
    let device_label = hardware_public_device_label(device_kind);
    div()
        .w_full()
        .flex()
        .items_center()
        .gap_2()
        .child(
            Spinner::new()
                .icon(IconName::LoaderCircle)
                .color(rgb(theme::TEXT_MUTED).into())
                .with_size(px(14.0)),
        )
        .child(app_muted_text(format!("Checking {device_label}...")))
}

pub(super) fn render_hardware_public_account_confirmation_wait(
    device_kind: HardwareDeviceKind,
    preview_address: Option<Address>,
) -> gpui::Div {
    let device_label = hardware_public_device_label(device_kind);
    div()
        .w_full()
        .p(px(12.0))
        .flex()
        .flex_col()
        .gap_2()
        .rounded_md()
        .border_1()
        .border_color(rgb(theme::BORDER))
        .bg(rgb_with_alpha(theme::SURFACE, 0.72))
        .child(app_strong_text(format!("Confirm on {device_label}")))
        .child(
            app_muted_text(format!(
                "Compare this address with the one shown on your {device_label}:"
            ))
            .whitespace_normal(),
        )
        .child(render_hardware_public_confirmation_address(preview_address))
        .child(app_muted_text("Only approve if the addresses match.").whitespace_normal())
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Spinner::new()
                        .icon(IconName::LoaderCircle)
                        .color(rgb(theme::SUCCESS).into())
                        .with_size(px(14.0)),
                )
                .child(
                    app_muted_text(format!("Waiting for {device_label} approval..."))
                        .text_color(rgb(theme::SUCCESS)),
                ),
        )
}

fn render_hardware_public_confirmation_address(preview_address: Option<Address>) -> gpui::Div {
    div()
        .w_full()
        .p(px(10.0))
        .rounded_md()
        .border_1()
        .border_color(rgb(theme::BORDER))
        .bg(rgb(theme::SURFACE))
        .child(
            app_muted_text(preview_address.map_or_else(
                || "Preparing address preview...".to_owned(),
                |address| format!("{address:#x}"),
            ))
            .font_family(APP_MONO_FONT_FAMILY)
            .text_size(APP_TEXT_SIZE)
            .whitespace_normal(),
        )
}

pub(in crate::root) fn hardware_public_account_setup_copy(
    device_kind: HardwareDeviceKind,
) -> String {
    let device_label = hardware_public_device_label(device_kind);
    format!(
        "Add a hardware-native Public EVM account from your {device_label}. The path is partitioned by the selected Private wallet account index. Confirm the receive address on your device before it is saved, and public transactions will require device approval."
    )
}

#[cfg(feature = "hardware")]
pub(super) async fn create_hardware_public_account(
    store: Arc<DesktopVaultStore>,
    view_session: Arc<DesktopViewSession>,
    device_kind: HardwareDeviceKind,
    label: String,
    trezor_app_passphrase: Option<Zeroizing<String>>,
    trezor_pin_matrix_provider: Option<TrezorPinMatrixProvider>,
    progress_tx: tokio::sync::mpsc::UnboundedSender<HardwarePublicAccountDerivationProgress>,
) -> Result<(PublicAccountMetadata, HardwareProfileSession), String> {
    let _ = progress_tx.send(HardwarePublicAccountDerivationProgress::CheckingDevice);
    let mut hardware_session = view_session
        .hardware_profile_session()
        .cloned()
        .ok_or_else(|| {
            "unlock the matching hardware profile before adding a hardware public account"
                .to_owned()
        })?;
    let account_index = store
        .next_derived_public_account_index_for_session(view_session.as_ref())
        .map_err(|error| error.to_string())?;
    let descriptor = HardwarePublicAccountDescriptor::for_wallet_public_index(
        device_kind,
        view_session.derivation_index(),
        account_index,
    )
    .map_err(|error| error.to_string())?;
    let confirmed_account = match device_kind {
        HardwareDeviceKind::Ledger => {
            let client = wallet_ops::hardware::ledger::LedgerHardwareDerivationClient::connect()
                .await
                .map_err(|error| error.to_string())?;
            let profile_path = parse_bip32_path(DEFAULT_HARDWARE_DERIVATION_PATH)
                .map_err(|error| error.to_string())?;
            let active = client
                .active_profile_session(&profile_path)
                .await
                .map_err(|error| error.to_string())?;
            ensure_hardware_public_profile_session(&hardware_session, &active)?;
            let preview = client
                .public_ethereum_address(&descriptor)
                .await
                .map_err(|error| error.to_string())?;
            let _ = progress_tx.send(
                HardwarePublicAccountDerivationProgress::AwaitingAddressConfirmation(preview),
            );
            let confirmed = client
                .confirmed_public_ethereum_account(&descriptor)
                .await
                .map_err(|error| error.to_string())?;
            ensure_hardware_public_confirmation_matches(preview, confirmed.address())?;
            confirmed
        }
        HardwareDeviceKind::Trezor => {
            let mut client =
                wallet_ops::hardware::trezor::TrezorHardwareDerivationClient::connect_with_session(
                    hardware_session.trezor_session_id.clone(),
                )
                .map_err(|error| error.to_string())?;
            client.set_passphrase_mode(hardware_session.trezor_passphrase_mode());
            if let Some(passphrase) = trezor_app_passphrase {
                client.set_app_passphrase_zeroizing(passphrase);
            }
            if let Some(provider) = trezor_pin_matrix_provider {
                client.set_pin_matrix_provider(provider);
            }
            let profile_path = parse_bip32_path(DEFAULT_HARDWARE_DERIVATION_PATH)
                .map_err(|error| error.to_string())?;
            let active = client
                .active_profile_session(&profile_path)
                .map_err(|error| error.to_string())?;
            ensure_hardware_public_profile_session(&hardware_session, &active)?;
            hardware_session
                .trezor_session_id
                .clone_from(&active.trezor_session_id);
            hardware_session.set_trezor_passphrase_mode(active.trezor_passphrase_mode());
            let preview = client
                .public_ethereum_address(&descriptor)
                .map_err(|error| error.to_string())?;
            let _ = progress_tx.send(
                HardwarePublicAccountDerivationProgress::AwaitingAddressConfirmation(preview),
            );
            let confirmed = client
                .confirmed_public_ethereum_account(&descriptor)
                .map_err(|error| error.to_string())?;
            ensure_hardware_public_confirmation_matches(preview, confirmed.address())?;
            confirmed
        }
    };
    let account = store
        .add_hardware_public_account(view_session.as_ref(), confirmed_account, Some(&label))
        .map_err(|error| error.to_string())?;
    Ok((account, hardware_session))
}

#[cfg(feature = "hardware")]
fn ensure_hardware_public_profile_session(
    expected: &HardwareProfileSession,
    actual: &HardwareProfileSession,
) -> Result<(), String> {
    if expected.device_kind == actual.device_kind && expected.binding == actual.binding {
        Ok(())
    } else {
        Err("wrong hardware device or passphrase context is active".to_owned())
    }
}

#[cfg(feature = "hardware")]
fn ensure_hardware_public_confirmation_matches(
    preview: Address,
    confirmed: Address,
) -> Result<(), String> {
    if preview == confirmed {
        Ok(())
    } else {
        Err(
            "Hardware wallet returned a different address than the preview. Account was not saved."
                .to_owned(),
        )
    }
}
