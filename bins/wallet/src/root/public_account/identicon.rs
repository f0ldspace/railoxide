use alloy::primitives::Address;
use gpui::{ParentElement, Pixels, Styled, div, px, rgb};
use ui::theme;

const PUBLIC_ACCOUNT_IDENTICON_SIZE: Pixels = px(40.0);
const PUBLIC_ACCOUNT_IDENTICON_CELL_SIZE: Pixels = px(8.0);
pub(in crate::root) const PUBLIC_ACCOUNT_IDENTICON_GRID_SIZE: usize = 5;
const PUBLIC_ACCOUNT_IDENTICON_SOURCE_COLUMNS: usize = 3;
pub(in crate::root) const PUBLIC_ACCOUNT_IDENTICON_CELL_COUNT: usize =
    PUBLIC_ACCOUNT_IDENTICON_GRID_SIZE * PUBLIC_ACCOUNT_IDENTICON_GRID_SIZE;
const PUBLIC_ACCOUNT_IDENTICON_COLORS: [u32; 8] = [
    theme::PRIMARY,
    theme::SUCCESS,
    theme::WARNING_STRONG,
    theme::WARNING,
    theme::DANGER,
    theme::PURPLE,
    theme::BLUE,
    theme::OLIVE,
];

pub(in crate::root) fn render_public_account_identicon(address: &Address) -> gpui::Div {
    let pattern = public_account_identicon_pattern(address);
    let foreground = public_account_identicon_color(address);
    let mut icon = div()
        .size(PUBLIC_ACCOUNT_IDENTICON_SIZE)
        .flex_none()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_0();
    for row in pattern.chunks_exact(PUBLIC_ACCOUNT_IDENTICON_GRID_SIZE) {
        let mut row_div = div().flex().gap_0();
        for active in row {
            let cell = div().size(PUBLIC_ACCOUNT_IDENTICON_CELL_SIZE);
            row_div = row_div.child(if *active {
                cell.bg(rgb(foreground))
            } else {
                cell
            });
        }
        icon = icon.child(row_div);
    }
    icon
}

pub(in crate::root) fn public_account_identicon_pattern(
    address: &Address,
) -> [bool; PUBLIC_ACCOUNT_IDENTICON_CELL_COUNT] {
    let mut pattern = [false; PUBLIC_ACCOUNT_IDENTICON_CELL_COUNT];
    let mut has_foreground = false;
    for (row_index, row) in pattern
        .chunks_exact_mut(PUBLIC_ACCOUNT_IDENTICON_GRID_SIZE)
        .enumerate()
    {
        for column in 0..PUBLIC_ACCOUNT_IDENTICON_SOURCE_COLUMNS {
            let bit_index = row_index * PUBLIC_ACCOUNT_IDENTICON_SOURCE_COLUMNS + column;
            let active = public_account_identicon_bit(address, bit_index);
            has_foreground |= active;
            row[column] = active;
            row[PUBLIC_ACCOUNT_IDENTICON_GRID_SIZE - column - 1] = active;
        }
    }
    if !has_foreground {
        pattern[PUBLIC_ACCOUNT_IDENTICON_CELL_COUNT / 2] = true;
    }
    pattern
}

fn public_account_identicon_bit(address: &Address, bit_index: usize) -> bool {
    let bytes = address.as_slice();
    let byte = bytes[(bit_index * 7) % bytes.len()];
    let shift = (bit_index * 5) % u8::BITS as usize;
    ((byte >> shift) & 1) == 1
}

pub(in crate::root) fn public_account_identicon_color(address: &Address) -> u32 {
    let bytes = address.as_slice();
    let color_index = usize::from(bytes[3] ^ bytes[7] ^ bytes[11] ^ bytes[15] ^ bytes[19])
        % PUBLIC_ACCOUNT_IDENTICON_COLORS.len();
    PUBLIC_ACCOUNT_IDENTICON_COLORS[color_index]
}
