//! Text-mode building blocks: bars, sparklines and fixed-width cells. Integer math only.

use borbatop_core::Permille;

const BAR_FILLED: char = '█';
const BAR_EMPTY: char = '░';
const SPARK_GLYPHS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
const ELLIPSIS: char = '…';
const FULL_SCALE: u64 = 1_000;
const HALF_SCALE: u64 = 500;
/// Usage from which a value is drawn as busy.
pub(crate) const BUSY_PERMILLE: u64 = 300;
/// Usage from which a value is drawn as hot.
pub(crate) const HOT_PERMILLE: u64 = 800;

/// How loud a value should look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Heat {
    Normal,
    Busy,
    Hot,
}

pub(crate) fn heat_of(permille: Permille) -> Heat {
    match permille.get() {
        value if value >= HOT_PERMILLE => Heat::Hot,
        value if value >= BUSY_PERMILLE => Heat::Busy,
        _ => Heat::Normal,
    }
}

/// Filled and empty cell counts of a bar `width` cells wide.
pub(crate) fn bar_cells(permille: u64, width: usize) -> (usize, usize) {
    let width_u64 = u64::try_from(width).unwrap_or(u64::MAX);
    let scaled = permille
        .min(FULL_SCALE)
        .saturating_mul(width_u64)
        .saturating_add(HALF_SCALE)
        / FULL_SCALE;
    let filled = usize::try_from(scaled).unwrap_or(width).min(width);
    (filled, width.saturating_sub(filled))
}

/// The filled and empty halves of a bar, as text.
pub(crate) fn bar_text(permille: u64, width: usize) -> (String, String) {
    let (filled, empty) = bar_cells(permille, width);
    (
        BAR_FILLED.to_string().repeat(filled),
        BAR_EMPTY.to_string().repeat(empty),
    )
}

/// The newest `width` values as a sparkline scaled to `max` (a zero `max` draws the floor).
pub(crate) fn sparkline<'a>(
    values: impl DoubleEndedIterator<Item = &'a u64> + ExactSizeIterator,
    max: u64,
    width: usize,
) -> String {
    let skip = values.len().saturating_sub(width);
    let top = SPARK_GLYPHS.len().saturating_sub(1);
    let top_u64 = u64::try_from(top).unwrap_or(0);

    let glyphs: String = values
        .skip(skip)
        .map(|&value| {
            let level = value
                .saturating_mul(top_u64)
                .checked_div(max)
                .unwrap_or(0)
                .min(top_u64);
            let position = usize::try_from(level).unwrap_or(0);
            SPARK_GLYPHS.get(position).copied().unwrap_or(BAR_EMPTY)
        })
        .collect();

    let missing = width.saturating_sub(glyphs.chars().count());
    format!("{}{glyphs}", " ".repeat(missing))
}

/// The largest value, at least 1, so a flat series still scales.
pub(crate) fn scale_of<'a>(values: impl Iterator<Item = &'a u64>) -> u64 {
    values.copied().max().unwrap_or(1).max(1)
}

/// `text` cut to `width` characters (ending in `…`) or padded with spaces on the right.
pub(crate) fn fit(text: &str, width: usize) -> String {
    let length = text.chars().count();
    if length <= width {
        return format!("{text}{}", " ".repeat(width.saturating_sub(length)));
    }
    let kept: String = text.chars().take(width.saturating_sub(1)).collect();
    if width == 0 {
        return kept;
    }
    format!("{kept}{ELLIPSIS}")
}

/// `text` cut to `width` characters and padded with spaces on the left.
pub(crate) fn fit_right(text: &str, width: usize) -> String {
    let length = text.chars().count();
    if length <= width {
        return format!("{}{text}", " ".repeat(width.saturating_sub(length)));
    }
    text.chars().take(width).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_rounds_to_the_nearest_cell_and_clamps() {
        assert_eq!(bar_cells(0, 10), (0, 10));
        assert_eq!(bar_cells(500, 10), (5, 5));
        assert_eq!(bar_cells(1_000, 10), (10, 0));
        assert_eq!(bar_cells(5_000, 10), (10, 0));
        assert_eq!(bar_cells(449, 10), (4, 6));
        assert_eq!(bar_cells(450, 10), (5, 5));
        assert_eq!(bar_cells(300, 0), (0, 0));
        assert_eq!(bar_text(250, 4), ("█".to_owned(), "░░░".to_owned()));
    }

    #[test]
    fn sparkline_scales_to_the_maximum_and_left_pads() {
        let values = [0_u64, 50, 100];
        assert_eq!(sparkline(values.iter(), 100, 3), "▁▄█");
        assert_eq!(sparkline(values.iter(), 100, 5), "  ▁▄█");
        assert_eq!(sparkline(values.iter(), 100, 2), "▄█");
        assert_eq!(sparkline(values.iter(), 0, 3), "▁▁▁");
        assert_eq!(sparkline([].iter(), 1, 2), "  ");
    }

    #[test]
    fn scale_is_never_zero() {
        assert_eq!(scale_of([0_u64, 0].iter()), 1);
        assert_eq!(scale_of([3_u64, 9, 4].iter()), 9);
        assert_eq!(scale_of([].iter()), 1);
    }

    #[test]
    fn fit_truncates_with_an_ellipsis_and_pads() {
        assert_eq!(fit("abc", 5), "abc  ");
        assert_eq!(fit("abcdef", 4), "abc…");
        assert_eq!(fit("ação longa", 5), "ação…");
        assert_eq!(fit("abc", 0), "");
        assert_eq!(fit_right("42", 5), "   42");
        assert_eq!(fit_right("123456", 4), "1234");
    }

    #[test]
    fn heat_thresholds() {
        assert_eq!(heat_of(Permille::new(299)), Heat::Normal);
        assert_eq!(heat_of(Permille::new(300)), Heat::Busy);
        assert_eq!(heat_of(Permille::new(800)), Heat::Hot);
    }
}
