//! Mouse input and the map of what sits under each cell of the last drawn frame.
//!
//! Drawing records where tabs, column titles and list rows were put; a click is then resolved
//! against that record, so the geometry lives in one place (the drawing code).

use crate::input::{SortKey, Tab};

/// A mouse action, independent of any terminal library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mouse {
    /// Left button pressed at a cell.
    Click {
        /// Zero-based column of the cell.
        column: u16,
        /// Zero-based row of the cell.
        row: u16,
    },
    /// Wheel turned away from the person.
    WheelUp,
    /// Wheel turned towards the person.
    WheelDown,
}

/// A horizontal run of cells on one row that stands for a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Run<T> {
    pub(crate) start: u16,
    pub(crate) end: u16,
    pub(crate) value: T,
}

impl<T: Copy> Run<T> {
    fn at(runs: &[Self], column: u16) -> Option<T> {
        runs.iter()
            .find(|run| (run.start..run.end).contains(&column))
            .map(|run| run.value)
    }
}

/// The rows of a list as they were drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ListRows {
    /// Screen row of the first list row.
    pub(crate) top: u16,
    /// Position, within the whole list, of the first row shown.
    pub(crate) first: usize,
    /// Number of rows shown.
    pub(crate) shown: usize,
}

/// What was drawn where, in the last frame.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct HitMap {
    pub(crate) tab_row: u16,
    pub(crate) tabs: Vec<Run<Tab>>,
    pub(crate) header_row: u16,
    pub(crate) sortable: Vec<Run<SortKey>>,
    pub(crate) list: Option<ListRows>,
}

impl HitMap {
    /// The tab drawn at a cell.
    pub(crate) fn tab_at(&self, column: u16, row: u16) -> Option<Tab> {
        if row != self.tab_row {
            return None;
        }
        Run::at(&self.tabs, column)
    }

    /// The sort column whose title is drawn at a cell.
    pub(crate) fn sort_at(&self, column: u16, row: u16) -> Option<SortKey> {
        if row != self.header_row {
            return None;
        }
        Run::at(&self.sortable, column)
    }

    /// The position, within the whole list, of the row drawn at screen row `row`.
    pub(crate) fn list_position_at(&self, row: u16) -> Option<usize> {
        let list = self.list?;
        let offset = usize::from(row.checked_sub(list.top)?);
        (offset < list.shown).then(|| list.first.saturating_add(offset))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> HitMap {
        HitMap {
            tab_row: 7,
            tabs: vec![
                Run {
                    start: 0,
                    end: 12,
                    value: Tab::Processes,
                },
                Run {
                    start: 13,
                    end: 24,
                    value: Tab::Services,
                },
            ],
            header_row: 9,
            sortable: vec![Run {
                start: 21,
                end: 27,
                value: SortKey::Cpu,
            }],
            list: Some(ListRows {
                top: 10,
                first: 5,
                shown: 3,
            }),
        }
    }

    #[test]
    fn tabs_are_found_by_column_on_their_row_only() {
        let map = map();
        assert_eq!(map.tab_at(0, 7), Some(Tab::Processes));
        assert_eq!(map.tab_at(11, 7), Some(Tab::Processes));
        assert_eq!(map.tab_at(12, 7), None, "the separator belongs to no tab");
        assert_eq!(map.tab_at(13, 7), Some(Tab::Services));
        assert_eq!(map.tab_at(5, 8), None);
    }

    #[test]
    fn sort_columns_are_found_on_the_header_row_only() {
        let map = map();
        assert_eq!(map.sort_at(21, 9), Some(SortKey::Cpu));
        assert_eq!(map.sort_at(26, 9), Some(SortKey::Cpu));
        assert_eq!(map.sort_at(27, 9), None);
        assert_eq!(map.sort_at(21, 10), None);
    }

    #[test]
    fn list_rows_map_to_positions_in_the_whole_list() {
        let map = map();
        assert_eq!(map.list_position_at(10), Some(5));
        assert_eq!(map.list_position_at(12), Some(7));
        assert_eq!(map.list_position_at(13), None, "below the last shown row");
        assert_eq!(map.list_position_at(9), None, "above the first row");
        assert_eq!(HitMap::default().list_position_at(0), None);
    }
}
