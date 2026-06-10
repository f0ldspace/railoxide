use wallet_ops::hardware::HardwareDeviceKind;

pub(in crate::root) const fn hardware_device_label(
    device_kind: HardwareDeviceKind,
) -> &'static str {
    match device_kind {
        HardwareDeviceKind::Ledger => "Ledger",
        HardwareDeviceKind::Trezor => "Trezor",
    }
}
