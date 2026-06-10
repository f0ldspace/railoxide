use super::error::HardwareDerivationError;

pub const DEFAULT_HARDWARE_DERIVATION_PATH: &str = "m/44'/60'/0'/0/0";
pub const HARDENED_BIP32_INDEX: u32 = 0x8000_0000;

pub(super) const fn hardened_bip32_index(index: u32) -> u32 {
    index | HARDENED_BIP32_INDEX
}

pub fn parse_bip32_path(path: &str) -> Result<Vec<u32>, HardwareDerivationError> {
    let path = path.trim();
    let path = path.strip_prefix("m/").unwrap_or(path);
    if path.is_empty() || path == "m" {
        return Err(HardwareDerivationError::InvalidPath(path.to_owned()));
    }
    path.split('/')
        .map(|segment| {
            let hardened =
                segment.ends_with('\'') || segment.ends_with('h') || segment.ends_with('H');
            let number = if hardened {
                &segment[..segment.len().saturating_sub(1)]
            } else {
                segment
            };
            let mut index = number
                .parse::<u32>()
                .map_err(|_| HardwareDerivationError::InvalidPath(segment.to_owned()))?;
            if hardened {
                index |= HARDENED_BIP32_INDEX;
            }
            Ok(index)
        })
        .collect()
}

#[must_use]
pub fn format_bip32_path(path: &[u32]) -> String {
    let mut formatted = String::from("m");
    for index in path {
        formatted.push('/');
        if index & HARDENED_BIP32_INDEX != 0 {
            formatted.push_str(&(index & 0x7fff_ffff).to_string());
            formatted.push('\'');
        } else {
            formatted.push_str(&index.to_string());
        }
    }
    formatted
}
