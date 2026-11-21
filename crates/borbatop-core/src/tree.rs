//! Process tree ordering and a bounded history ring for sparklines.

use std::collections::{HashMap, VecDeque};

use crate::process::Pid;

/// A process placed in tree order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeRow {
    index: usize,
    depth: usize,
}

impl TreeRow {
    /// Position of the process in the slice given to [`order_as_tree`].
    #[must_use]
    pub const fn index(&self) -> usize {
        self.index
    }

    /// Distance from the nearest root: 0 for a root, 1 for its children, and so on.
    #[must_use]
    pub const fn depth(&self) -> usize {
        self.depth
    }
}

/// Orders processes so that every child follows its parent, siblings sorted by PID.
///
/// A process whose parent is not in `links` (kernel threads, init, a parent that already
/// exited) becomes a root. Cycles cannot occur in a real snapshot, but any process left
/// unreachable is appended as a root so none is ever dropped.
///
/// # Arguments
///
/// * `links` - `(pid, parent pid)` of each process, in the caller's own order
///
/// # Returns
///
/// One row per input entry, each exactly once.
///
/// # Examples
///
/// ```
/// use borbatop_core::{order_as_tree, Pid};
///
/// let links = [
///     (Pid::new(30), Pid::new(10)),
///     (Pid::new(10), Pid::new(1)),
///     (Pid::new(20), Pid::new(10)),
/// ];
/// let order: Vec<(usize, usize)> = order_as_tree(&links)
///     .iter()
///     .map(|row| (row.index(), row.depth()))
///     .collect();
/// assert_eq!(order, [(1, 0), (2, 1), (0, 1)]); // 10, then children 20 and 30
/// ```
#[must_use]
pub fn order_as_tree(links: &[(Pid, Pid)]) -> Vec<TreeRow> {
    let by_pid: HashMap<Pid, usize> = links
        .iter()
        .enumerate()
        .map(|(index, &(pid, _))| (pid, index))
        .collect();

    let mut children: Vec<Vec<usize>> = vec![Vec::new(); links.len()];
    let mut roots: Vec<usize> = Vec::with_capacity(links.len());
    for (index, &(pid, parent)) in links.iter().enumerate() {
        let parent_index = by_pid.get(&parent).filter(|_| parent != pid);
        match parent_index.and_then(|&parent_index| children.get_mut(parent_index)) {
            Some(siblings) => siblings.push(index),
            None => roots.push(index),
        }
    }

    let pid_of = |index: &usize| links.get(*index).map(|&(pid, _)| pid);
    roots.sort_unstable_by_key(pid_of);
    for siblings in &mut children {
        siblings.sort_unstable_by_key(pid_of);
    }

    let mut visited = vec![false; links.len()];
    let mut rows = Vec::with_capacity(links.len());
    for root in roots {
        walk(root, &children, &mut visited, &mut rows);
    }
    for orphan in 0..links.len() {
        if !visited.get(orphan).copied().unwrap_or(true) {
            walk(orphan, &children, &mut visited, &mut rows);
        }
    }

    rows
}

fn walk(root: usize, children: &[Vec<usize>], visited: &mut [bool], rows: &mut Vec<TreeRow>) {
    let mut stack = vec![(root, 0_usize)];

    while let Some((index, depth)) = stack.pop() {
        let Some(seen) = visited.get_mut(index) else {
            continue;
        };
        if std::mem::replace(seen, true) {
            continue;
        }
        rows.push(TreeRow { index, depth });
        if let Some(siblings) = children.get(index) {
            let next_depth = depth.saturating_add(1);
            stack.extend(siblings.iter().rev().map(|&child| (child, next_depth)));
        }
    }
}

/// The most recent values of a series, oldest first, bounded in size.
///
/// # Examples
///
/// ```
/// use borbatop_core::History;
///
/// let mut history = History::new(2);
/// history.push(1);
/// history.push(2);
/// history.push(3);
/// assert_eq!(history.iter().copied().collect::<Vec<_>>(), [2, 3]);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History<T> {
    values: VecDeque<T>,
    capacity: usize,
}

impl<T> History<T> {
    /// Creates an empty history that keeps at most `capacity` values.
    ///
    /// # Arguments
    ///
    /// * `capacity` - number of values kept; the buffer is allocated once, up front
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            values: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Appends a value, dropping the oldest one when full.
    ///
    /// # Arguments
    ///
    /// * `value` - the newest value
    pub fn push(&mut self, value: T) {
        if self.capacity == 0 {
            return;
        }
        if self.values.len() >= self.capacity {
            self.values.pop_front();
        }
        self.values.push_back(value);
    }

    /// Values from oldest to newest.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> + ExactSizeIterator {
        self.values.iter()
    }

    /// The newest value, if any.
    #[must_use]
    pub fn latest(&self) -> Option<&T> {
        self.values.back()
    }

    /// Number of values currently kept.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether no value has been pushed yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn pid(raw: u32) -> Pid {
        Pid::new(raw)
    }

    #[test]
    fn children_follow_their_parent_with_depth() {
        let links = [
            (pid(1), pid(0)),
            (pid(5), pid(1)),
            (pid(3), pid(1)),
            (pid(9), pid(5)),
        ];
        let order: Vec<(u32, usize)> = order_as_tree(&links)
            .iter()
            .map(|row| (links[row.index()].0.get(), row.depth()))
            .collect();
        assert_eq!(order, [(1, 0), (3, 1), (5, 1), (9, 2)]);
    }

    #[test]
    fn missing_parent_makes_a_root_and_self_parent_is_not_a_loop() {
        let links = [(pid(7), pid(999)), (pid(8), pid(8))];
        let rows = order_as_tree(&links);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row.depth() == 0));
    }

    #[test]
    fn a_parent_cycle_still_lists_every_process_once() {
        let links = [(pid(1), pid(2)), (pid(2), pid(1))];
        let rows = order_as_tree(&links);
        assert_eq!(rows.len(), 2);
        let mut seen: Vec<usize> = rows.iter().map(TreeRow::index).collect();
        seen.sort_unstable();
        assert_eq!(seen, [0, 1]);
    }

    #[test]
    fn empty_input_gives_empty_tree() {
        assert_eq!(order_as_tree(&[]).len(), 0);
    }

    #[test]
    fn history_is_bounded_and_ordered() {
        let mut history = History::new(3);
        assert!(history.is_empty());
        for value in 1..=5 {
            history.push(value);
        }
        assert_eq!(history.len(), 3);
        assert_eq!(history.latest(), Some(&5));
        assert_eq!(history.iter().copied().collect::<Vec<_>>(), [3, 4, 5]);
    }

    #[test]
    fn zero_capacity_history_stays_empty() {
        let mut history = History::new(0);
        history.push(1);
        assert!(history.is_empty());
        assert_eq!(history.latest(), None);
    }
}
