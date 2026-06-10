use wallet_ops::hardware::trezor::TrezorPinMatrixRequestKind;

pub(super) const fn trezor_pin_matrix_title(kind: TrezorPinMatrixRequestKind) -> &'static str {
    match kind {
        TrezorPinMatrixRequestKind::Current => "Enter Trezor PIN",
        TrezorPinMatrixRequestKind::NewFirst => "Enter new Trezor PIN",
        TrezorPinMatrixRequestKind::NewSecond => "Confirm new Trezor PIN",
        TrezorPinMatrixRequestKind::WipeCodeFirst => "Enter Trezor wipe code",
        TrezorPinMatrixRequestKind::WipeCodeSecond => "Confirm Trezor wipe code",
    }
}
