//! Screen state and key handling: no terminal, no drawing, fully testable.

use std::cell::Cell;
use std::cmp::Ordering;
use std::collections::HashSet;

use borbatop_core::{ActiveState, Pid, ProcessId, UnitRow, order_as_tree};

use crate::alerts::{Alert, AlertThresholds, evaluate};
use crate::dialog::{Dialog, DialogOutcome};
use crate::input::{Command, Key, SortKey, Tab};
use crate::model::{Model, PacmanView, ProcessRow};
use crate::theme::ThemeKind;
use crate::widgets::safe;

const MESSAGE_TTL_TICKS: u8 = 4;
const PAGE_ROWS: isize = 10;
const DEFAULT_HISTORY_LEN: usize = 60;
const FIRST_TAB_DIGIT: u32 = 1;

/// How the screen starts out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiConfig {
    /// Palette to draw with.
    pub theme: ThemeKind,
    /// Start with processes shown as a tree.
    pub tree: bool,
    /// Show the owning package column.
    pub show_package: bool,
    /// Number of samples kept for sparklines.
    pub history_len: usize,
    /// Levels from which a reading raises an alert.
    pub alerts: AlertThresholds,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: ThemeKind::Borba,
            tree: false,
            show_package: true,
            history_len: DEFAULT_HISTORY_LEN,
            alerts: AlertThresholds::default(),
        }
    }
}

/// Importance of a status message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    /// Informational.
    Info,
    /// Something failed.
    Error,
}

/// Whether new samples are shown or the display is frozen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Playback {
    Live,
    Paused,
}

/// Whether keys edit the filter or drive the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputMode {
    Normal,
    Filtering,
}

/// How the process table is arranged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcessLayout {
    Flat,
    Tree,
}

/// Whether an optional column is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Visibility {
    Shown,
    Hidden,
}

impl Visibility {
    const fn toggled(self) -> Self {
        match self {
            Self::Shown => Self::Hidden,
            Self::Hidden => Self::Shown,
        }
    }
}

/// Progress of the Pacman report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PacmanState {
    NotRequested,
    Loading,
    Loaded(PacmanView),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Message {
    pub(crate) text: String,
    pub(crate) kind: MessageKind,
    ttl: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LogView {
    pub(crate) unit: String,
    pub(crate) lines: Vec<String>,
    pub(crate) failed: bool,
    /// Lines scrolled up from the newest one; 0 shows the end of the journal.
    pub(crate) scroll: usize,
}

/// A process row that survived filtering, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VisibleRow {
    pub(crate) index: usize,
    pub(crate) depth: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Nav {
    Up,
    Down,
    PageUp,
    PageDown,
    Top,
    Bottom,
}

/// All state of the screen.
#[derive(Debug)]
pub struct App {
    pub(crate) model: Model,
    pub(crate) config: UiConfig,
    pub(crate) tab: Tab,
    pub(crate) sort: SortKey,
    pub(crate) descending: bool,
    pub(crate) layout: ProcessLayout,
    pub(crate) package_column: Visibility,
    pub(crate) selected_pid: Option<Pid>,
    pub(crate) selected_unit: Option<String>,
    pub(crate) filter: String,
    pub(crate) input: InputMode,
    pub(crate) playback: Playback,
    pub(crate) dialog: Option<Dialog>,
    pub(crate) logs: Option<LogView>,
    pub(crate) message: Option<Message>,
    pub(crate) alerts: Vec<Alert>,
    pub(crate) pacman: PacmanState,
    pub(crate) scroll: usize,
    /// Largest useful scroll offset, learned while drawing.
    pub(crate) max_scroll: Cell<usize>,
}

impl App {
    /// A screen showing an empty model, sorted by CPU.
    ///
    /// # Arguments
    ///
    /// * `config` - initial options
    #[must_use]
    pub fn new(config: UiConfig) -> Self {
        Self {
            model: Model::empty(config.history_len),
            config,
            tab: Tab::Processes,
            sort: SortKey::Cpu,
            descending: SortKey::Cpu.descending_by_default(),
            layout: if config.tree {
                ProcessLayout::Tree
            } else {
                ProcessLayout::Flat
            },
            package_column: if config.show_package {
                Visibility::Shown
            } else {
                Visibility::Hidden
            },
            selected_pid: None,
            selected_unit: None,
            filter: String::new(),
            input: InputMode::Normal,
            playback: Playback::Live,
            dialog: None,
            logs: None,
            message: None,
            alerts: Vec::new(),
            pacman: PacmanState::NotRequested,
            scroll: 0,
            max_scroll: Cell::new(0),
        }
    }

    /// Replaces the shown data, unless the display is paused.
    ///
    /// # Arguments
    ///
    /// * `model` - the newest sample
    pub fn apply_model(&mut self, model: Model) {
        if self.playback == Playback::Live {
            self.model = model;
            self.refresh_alerts();
        }
        self.ensure_selection();
    }

    /// Re-evaluates the alerts and announces the ones that just appeared.
    fn refresh_alerts(&mut self) {
        let current = evaluate(&self.model, &self.config.alerts);
        let announced = current
            .iter()
            .find(|alert| !self.alerts.iter().any(|known| known.kind == alert.kind))
            .map(ToString::to_string);
        self.alerts = current;
        if let Some(text) = announced {
            self.notify(format!("alert: {text}"), MessageKind::Error);
        }
    }

    /// The conditions currently above their thresholds.
    #[must_use]
    pub fn alerts(&self) -> &[Alert] {
        &self.alerts
    }

    /// Stores a freshly built Pacman report.
    ///
    /// # Arguments
    ///
    /// * `view` - the report
    pub fn set_pacman(&mut self, view: PacmanView) {
        self.pacman = PacmanState::Loaded(view);
    }

    /// Opens the journal overlay for `unit`.
    ///
    /// # Arguments
    ///
    /// * `unit` - unit the text belongs to
    /// * `result` - journal text, or why it could not be read
    pub fn set_logs(&mut self, unit: String, result: Result<String, String>) {
        let (lines, failed) = match result {
            Ok(text) => (text.lines().map(safe).collect(), false),
            Err(reason) => (vec![safe(&reason)], true),
        };
        self.logs = Some(LogView {
            unit,
            lines,
            failed,
            scroll: 0,
        });
    }

    /// Shows a message in the status bar for a few ticks.
    ///
    /// # Arguments
    ///
    /// * `text` - message to show
    /// * `kind` - how important it is
    pub fn notify(&mut self, text: impl Into<String>, kind: MessageKind) {
        self.message = Some(Message {
            text: text.into(),
            kind,
            ttl: MESSAGE_TTL_TICKS,
        });
    }

    /// Advances time: messages fade after a few ticks. Call once per refresh interval.
    pub fn tick(&mut self) {
        self.message = self.message.take().and_then(|mut message| {
            message.ttl = message.ttl.saturating_sub(1);
            (message.ttl > 0).then_some(message)
        });
    }

    /// The sample currently shown.
    #[must_use]
    pub const fn model(&self) -> &Model {
        &self.model
    }

    /// Whether the display is frozen.
    #[must_use]
    pub const fn is_paused(&self) -> bool {
        matches!(self.playback, Playback::Paused)
    }

    /// Whether the Pacman report is being rebuilt.
    #[must_use]
    pub const fn is_pacman_loading(&self) -> bool {
        matches!(self.pacman, PacmanState::Loading)
    }

    /// The tab being shown.
    #[must_use]
    pub const fn tab(&self) -> Tab {
        self.tab
    }

    /// Applies a key press and says what, if anything, the program must do.
    ///
    /// # Arguments
    ///
    /// * `key` - the key that was pressed
    pub fn handle_key(&mut self, key: Key) -> Option<Command> {
        let command = self.dispatch_key(key);
        self.ensure_selection();
        command
    }

    /// Keeps a process and a unit selected whenever there is one to select.
    fn ensure_selection(&mut self) {
        let rows = self.visible_processes();
        if self.selected_row_position(&rows).is_none() {
            self.selected_pid = rows
                .first()
                .and_then(|row| self.model.processes.get(row.index))
                .map(|process| process.pid);
        }
        let units = self.visible_units();
        let still_listed = units
            .iter()
            .any(|unit| Some(unit.name()) == self.selected_unit.as_deref());
        if !still_listed {
            self.selected_unit = units.first().map(|unit| unit.name().to_owned());
        }
    }

    fn dispatch_key(&mut self, key: Key) -> Option<Command> {
        if self.dialog.is_some() {
            return self.handle_dialog_key(key);
        }
        if self.logs.is_some() {
            self.handle_logs_key(key);
            return None;
        }
        if self.input == InputMode::Filtering {
            self.handle_filter_key(key);
            return None;
        }
        match key {
            Key::Char(character) => self.handle_char(character),
            Key::Up => self.navigate(Nav::Up),
            Key::Down => self.navigate(Nav::Down),
            Key::PageUp => self.navigate(Nav::PageUp),
            Key::PageDown => self.navigate(Nav::PageDown),
            Key::Home => self.navigate(Nav::Top),
            Key::End => self.navigate(Nav::Bottom),
            Key::Tab => self.select_tab(Tab::wrapping(self.tab_offset(1))),
            Key::BackTab => self.select_tab(Tab::wrapping(self.tab_offset(-1))),
            Key::Escape => {
                self.filter.clear();
                None
            }
            Key::Enter | Key::Backspace => None,
        }
    }

    fn tab_offset(&self, delta: isize) -> isize {
        isize::try_from(self.tab.index())
            .unwrap_or(0)
            .saturating_add(delta)
    }

    fn handle_char(&mut self, character: char) -> Option<Command> {
        match character {
            'q' => Some(Command::Quit),
            'j' => self.navigate(Nav::Down),
            'k' => self.navigate(Nav::Up),
            'g' => self.navigate(Nav::Top),
            'G' => self.navigate(Nav::Bottom),
            's' => {
                self.sort = self.sort.next();
                self.descending = self.sort.descending_by_default();
                None
            }
            'r' => {
                self.descending = !self.descending;
                None
            }
            't' => {
                self.layout = match self.layout {
                    ProcessLayout::Flat => ProcessLayout::Tree,
                    ProcessLayout::Tree => ProcessLayout::Flat,
                };
                None
            }
            'p' => {
                self.package_column = self.package_column.toggled();
                None
            }
            ' ' => {
                self.playback = match self.playback {
                    Playback::Live => Playback::Paused,
                    Playback::Paused => Playback::Live,
                };
                None
            }
            '/' => {
                if matches!(self.tab, Tab::Processes | Tab::Services) {
                    self.input = InputMode::Filtering;
                }
                None
            }
            'K' => self.open_signal_dialog(),
            'n' => self.open_nice_dialog(),
            'c' => self.open_affinity_dialog(),
            'a' => self.open_service_dialog(),
            'R' => self.request_pacman(),
            'l' => self.request_logs(),
            digit => {
                let position = digit
                    .to_digit(10)
                    .and_then(|value| value.checked_sub(FIRST_TAB_DIGIT))
                    .and_then(|value| usize::try_from(value).ok())
                    .and_then(|value| Tab::ALL.get(value).copied());
                position.and_then(|tab| self.select_tab(tab))
            }
        }
    }

    fn select_tab(&mut self, tab: Tab) -> Option<Command> {
        if tab != self.tab {
            self.scroll = 0;
        }
        self.tab = tab;
        if tab == Tab::Pacman && self.pacman == PacmanState::NotRequested {
            return self.request_pacman();
        }
        None
    }

    fn request_pacman(&mut self) -> Option<Command> {
        if self.tab != Tab::Pacman || self.is_pacman_loading() {
            return None;
        }
        self.pacman = PacmanState::Loading;
        Some(Command::RefreshPacman)
    }

    fn request_logs(&mut self) -> Option<Command> {
        if self.tab != Tab::Services {
            return None;
        }
        self.selected_unit
            .clone()
            .map(|unit| Command::LoadLogs { unit })
    }

    /// The selected process, when the Processes tab is showing.
    fn selected_process(&self) -> Option<&ProcessRow> {
        if self.tab != Tab::Processes {
            return None;
        }
        self.model
            .processes
            .iter()
            .find(|process| Some(process.pid) == self.selected_pid)
    }

    fn open_signal_dialog(&mut self) -> Option<Command> {
        let process = self.selected_process()?;
        let id = ProcessId::new(process.pid, process.start_time);
        self.dialog = Some(Dialog::signal(id, &process.command));
        None
    }

    fn open_nice_dialog(&mut self) -> Option<Command> {
        let process = self.selected_process()?;
        let id = ProcessId::new(process.pid, process.start_time);
        self.dialog = Some(Dialog::nice(id, &process.command, process.nice));
        None
    }

    fn open_affinity_dialog(&mut self) -> Option<Command> {
        let cpu_count = self.model.cores.len();
        let process = self.selected_process()?;
        if cpu_count == 0 {
            return None;
        }
        let id = ProcessId::new(process.pid, process.start_time);
        self.dialog = Some(Dialog::affinity(id, &process.command, cpu_count));
        None
    }

    fn open_service_dialog(&mut self) -> Option<Command> {
        if self.tab != Tab::Services {
            return None;
        }
        let unit = self.selected_unit.as_deref()?;
        self.dialog = Some(Dialog::service(unit));
        None
    }

    fn handle_dialog_key(&mut self, key: Key) -> Option<Command> {
        let dialog = self.dialog.as_mut()?;
        match dialog.handle_key(key) {
            DialogOutcome::Open => None,
            DialogOutcome::Closed => {
                self.dialog = None;
                None
            }
            DialogOutcome::Confirmed(command) => {
                self.dialog = None;
                Some(command)
            }
        }
    }

    fn handle_logs_key(&mut self, key: Key) {
        if key == Key::Escape || key == Key::Char('q') {
            self.logs = None;
            return;
        }
        let Some(logs) = self.logs.as_mut() else {
            return;
        };
        let last = logs.lines.len().saturating_sub(1);
        match nav_of(key) {
            Some(Nav::Up) => logs.scroll = logs.scroll.saturating_add(1).min(last),
            Some(Nav::Down) => logs.scroll = logs.scroll.saturating_sub(1),
            Some(Nav::PageUp) => logs.scroll = logs.scroll.saturating_add(PAGE_STEP).min(last),
            Some(Nav::PageDown) => logs.scroll = logs.scroll.saturating_sub(PAGE_STEP),
            Some(Nav::Top) => logs.scroll = last,
            Some(Nav::Bottom) => logs.scroll = 0,
            None => {}
        }
    }

    fn handle_filter_key(&mut self, key: Key) {
        if let Key::Char(character) = key {
            self.filter.push(character);
        } else if key == Key::Backspace {
            self.filter.pop();
        } else if key == Key::Enter {
            self.input = InputMode::Normal;
        } else if key == Key::Escape {
            self.filter.clear();
            self.input = InputMode::Normal;
        }
    }

    fn navigate(&mut self, nav: Nav) -> Option<Command> {
        let delta = match nav {
            Nav::Up => -1,
            Nav::Down => 1,
            Nav::PageUp => -PAGE_ROWS,
            Nav::PageDown => PAGE_ROWS,
            Nav::Top => isize::MIN,
            Nav::Bottom => isize::MAX,
        };
        match self.tab {
            Tab::Processes => self.move_process_selection(delta),
            Tab::Services => self.move_service_selection(delta),
            Tab::Pacman | Tab::Io => {
                self.scroll = self
                    .scroll
                    .saturating_add_signed(delta)
                    .min(self.max_scroll.get());
            }
        }
        None
    }

    fn move_process_selection(&mut self, delta: isize) {
        let rows = self.visible_processes();
        let Some(last) = rows.len().checked_sub(1) else {
            return;
        };
        let current = self.selected_row_position(&rows).unwrap_or(0);
        let target = current.saturating_add_signed(delta).min(last);
        self.selected_pid = rows
            .get(target)
            .and_then(|row| self.model.processes.get(row.index))
            .map(|process| process.pid);
    }

    fn move_service_selection(&mut self, delta: isize) {
        let units = self.visible_units();
        let Some(last) = units.len().checked_sub(1) else {
            return;
        };
        let current = units
            .iter()
            .position(|unit| Some(unit.name()) == self.selected_unit.as_deref())
            .unwrap_or(0);
        let target = current.saturating_add_signed(delta).min(last);
        self.selected_unit = units.get(target).map(|unit| unit.name().to_owned());
    }

    /// Position of the selected process within `rows`.
    pub(crate) fn selected_row_position(&self, rows: &[VisibleRow]) -> Option<usize> {
        rows.iter().position(|row| {
            self.model
                .processes
                .get(row.index)
                .map(|process| process.pid)
                == self.selected_pid
        })
    }

    /// Processes that pass the filter, in display order (tree order or sorted).
    pub(crate) fn visible_processes(&self) -> Vec<VisibleRow> {
        let needle = self.filter.trim().to_lowercase();
        let matching: HashSet<usize> = self
            .model
            .processes
            .iter()
            .enumerate()
            .filter(|(_, process)| needle.is_empty() || matches_filter(process, &needle))
            .map(|(index, _)| index)
            .collect();

        if self.layout == ProcessLayout::Tree {
            let links: Vec<(Pid, Pid)> = self
                .model
                .processes
                .iter()
                .map(|process| (process.pid, process.parent))
                .collect();
            return order_as_tree(&links)
                .into_iter()
                .filter(|row| matching.contains(&row.index()))
                .map(|row| VisibleRow {
                    index: row.index(),
                    depth: row.depth(),
                })
                .collect();
        }

        let mut indices: Vec<usize> = matching.into_iter().collect();
        indices.sort_by(|left, right| self.compare(*left, *right));
        indices
            .into_iter()
            .map(|index| VisibleRow { index, depth: 0 })
            .collect()
    }

    fn compare(&self, left: usize, right: usize) -> Ordering {
        let (Some(left), Some(right)) = (
            self.model.processes.get(left),
            self.model.processes.get(right),
        ) else {
            return Ordering::Equal;
        };
        let ordering = match self.sort {
            SortKey::Cpu => left.cpu.cmp(&right.cpu),
            SortKey::Memory => left.memory.cmp(&right.memory),
            SortKey::Pid => left.pid.cmp(&right.pid),
            SortKey::Command => left.command.cmp(&right.command),
        };
        let ordering = if self.descending {
            ordering.reverse()
        } else {
            ordering
        };
        ordering.then_with(|| left.pid.cmp(&right.pid))
    }

    /// Units that pass the filter, failed ones first.
    pub(crate) fn visible_units(&self) -> Vec<&UnitRow> {
        let Ok(list) = &self.model.services else {
            return Vec::new();
        };
        let needle = self.filter.trim().to_lowercase();
        let mut units: Vec<&UnitRow> = list
            .units()
            .iter()
            .filter(|unit| {
                needle.is_empty()
                    || unit.name().to_lowercase().contains(&needle)
                    || unit.description().to_lowercase().contains(&needle)
            })
            .collect();
        units.sort_by_key(|unit| (unit.active() != ActiveState::Failed, unit.name().to_owned()));
        units
    }
}

const PAGE_STEP: usize = 10;

fn nav_of(key: Key) -> Option<Nav> {
    match key {
        Key::Up | Key::Char('k') => Some(Nav::Up),
        Key::Down | Key::Char('j') => Some(Nav::Down),
        Key::PageUp => Some(Nav::PageUp),
        Key::PageDown => Some(Nav::PageDown),
        Key::Home | Key::Char('g') => Some(Nav::Top),
        Key::End | Key::Char('G') => Some(Nav::Bottom),
        Key::Char(_) | Key::Enter | Key::Escape | Key::Backspace | Key::Tab | Key::BackTab => None,
    }
}

fn matches_filter(process: &ProcessRow, needle: &str) -> bool {
    process.command.to_lowercase().contains(needle)
        || process.user.to_lowercase().contains(needle)
        || process.unit.to_lowercase().contains(needle)
        || process.package.to_lowercase().contains(needle)
        || process.pid.to_string().contains(needle)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use borbatop_core::{Bytes, Permille, ProcessState, parse_systemctl_units};

    use crate::input::SignalChoice;

    use super::*;

    fn row(pid: u32, parent: u32, cpu: u64, command: &str) -> ProcessRow {
        let stat = format!("{pid} (x) S {parent} 1 1 0 -1 0 0 0 0 0 0 0 0 0 20 0 3 0 1 1 1 0");
        let sample = borbatop_core::parse_process_stat(&stat).unwrap();
        ProcessRow {
            pid: sample.pid(),
            start_time: sample.id().start_time(),
            parent: sample.parent(),
            user: "borba".to_owned(),
            state: ProcessState::Sleeping,
            cpu: Permille::new(cpu),
            memory: Bytes::new(cpu.saturating_mul(10)),
            nice: sample.nice(),
            threads: sample.threads(),
            unit: "app.scope".to_owned(),
            package: "pkg".to_owned(),
            command: command.to_owned(),
        }
    }

    fn app_with(rows: Vec<ProcessRow>) -> App {
        let mut app = App::new(UiConfig::default());
        let mut model = Model::empty(4);
        model.processes = rows;
        app.apply_model(model);
        app
    }

    fn pids(app: &App) -> Vec<u32> {
        app.visible_processes()
            .iter()
            .map(|row| app.model.processes[row.index].pid.get())
            .collect()
    }

    #[test]
    fn rows_are_sorted_by_cpu_then_sort_key_cycles() {
        let mut app = app_with(vec![
            row(1, 0, 10, "a"),
            row(2, 0, 900, "b"),
            row(3, 0, 50, "c"),
        ]);
        assert_eq!(pids(&app), [2, 3, 1]);

        app.handle_key(Key::Char('r'));
        assert_eq!(pids(&app), [1, 3, 2]);

        app.handle_key(Key::Char('s')); // memory, descending
        assert_eq!(pids(&app), [2, 3, 1]);
        app.handle_key(Key::Char('s')); // pid, ascending
        assert_eq!(pids(&app), [1, 2, 3]);
        app.handle_key(Key::Char('s')); // command, ascending
        assert_eq!(pids(&app), [1, 2, 3]);
    }

    #[test]
    fn selection_follows_the_process_not_the_row() {
        let mut app = app_with(vec![
            row(1, 0, 10, "a"),
            row(2, 0, 900, "b"),
            row(3, 0, 50, "c"),
        ]);
        app.handle_key(Key::Down);
        assert_eq!(app.selected_pid, Some(Pid::new(3)));

        let mut model = app.model.clone();
        model.processes = vec![row(1, 0, 999, "a"), row(2, 0, 900, "b"), row(3, 0, 50, "c")];
        app.apply_model(model);

        let rows = app.visible_processes();
        assert_eq!(app.selected_row_position(&rows), Some(2));
        app.handle_key(Key::Char('G'));
        assert_eq!(app.selected_pid, Some(Pid::new(3)));
        app.handle_key(Key::Char('g'));
        assert_eq!(app.selected_pid, Some(Pid::new(1)));
        app.handle_key(Key::PageDown);
        assert_eq!(app.selected_pid, Some(Pid::new(3)));
    }

    #[test]
    fn filter_matches_any_column_and_can_be_cleared() {
        let mut app = app_with(vec![row(10, 0, 5, "firefox"), row(11, 0, 5, "rustc")]);
        app.handle_key(Key::Char('/'));
        for character in "RUST".chars() {
            app.handle_key(Key::Char(character));
        }
        assert_eq!(pids(&app), [11]);
        app.handle_key(Key::Enter);
        assert_eq!(app.input, InputMode::Normal);
        assert_eq!(pids(&app), [11]);

        app.handle_key(Key::Escape);
        assert_eq!(pids(&app), [10, 11]);

        app.handle_key(Key::Char('/'));
        app.handle_key(Key::Char('1'));
        app.handle_key(Key::Char('1'));
        assert_eq!(pids(&app), [11]);
        app.handle_key(Key::Backspace);
        assert_eq!(pids(&app), [10, 11]);
        app.handle_key(Key::Escape);
        assert_eq!(app.input, InputMode::Normal);
    }

    #[test]
    fn tree_mode_keeps_children_under_parents() {
        let mut app = app_with(vec![
            row(5, 1, 1, "child"),
            row(1, 0, 1, "init"),
            row(9, 5, 1, "grandchild"),
        ]);
        app.handle_key(Key::Char('t'));
        let rows = app.visible_processes();
        let shape: Vec<(u32, usize)> = rows
            .iter()
            .map(|row| (app.model.processes[row.index].pid.get(), row.depth))
            .collect();
        assert_eq!(shape, [(1, 0), (5, 1), (9, 2)]);
    }

    #[test]
    fn signal_dialog_confirms_the_chosen_signal() {
        let mut app = app_with(vec![row(7, 0, 5, "victim")]);
        app.handle_key(Key::Char('j')); // selects the only row
        assert_eq!(app.handle_key(Key::Char('K')), None);
        assert!(app.dialog.is_some());

        app.handle_key(Key::Down);
        app.handle_key(Key::Down);
        app.handle_key(Key::Down); // clamps at SIGKILL
        let command = app.handle_key(Key::Enter);

        assert_eq!(
            command,
            Some(Command::SendSignal {
                id: ProcessId::new(Pid::new(7), borbatop_core::StartTime::new(1)),
                signal: SignalChoice::Kill
            })
        );
        assert!(app.dialog.is_none());
    }

    #[test]
    fn signal_dialog_can_be_cancelled() {
        let mut app = app_with(vec![row(7, 0, 5, "victim")]);
        app.handle_key(Key::Down);
        app.handle_key(Key::Char('K'));
        assert_eq!(app.handle_key(Key::Escape), None);
        assert!(app.dialog.is_none());
        assert_eq!(app.handle_key(Key::Char('q')), Some(Command::Quit));
    }

    #[test]
    fn dialog_needs_a_process_and_the_process_tab() {
        let mut empty = app_with(vec![]);
        empty.handle_key(Key::Char('K'));
        assert!(empty.dialog.is_none());

        let mut app = app_with(vec![row(7, 0, 5, "victim")]);
        app.handle_key(Key::Char('2'));
        app.handle_key(Key::Char('K'));
        assert!(app.dialog.is_none());
    }

    #[test]
    fn a_new_alert_is_listed_and_announced_only_once() {
        let mut app = app_with(vec![]);
        let mut model = Model::empty(4);
        model.cpu_history.push(990);

        app.apply_model(model.clone());
        assert_eq!(app.alerts().len(), 1);
        let announced = app.message.take().unwrap();
        assert!(
            announced.text.starts_with("alert: cpu"),
            "{}",
            announced.text
        );
        assert_eq!(announced.kind, MessageKind::Error);

        app.apply_model(model);
        assert!(
            app.message.is_none(),
            "an alert that is already known is silent"
        );

        app.apply_model(Model::empty(4));
        assert_eq!(app.alerts().len(), 0);
    }

    #[test]
    fn paused_display_keeps_its_alerts() {
        let mut app = app_with(vec![]);
        app.handle_key(Key::Char(' '));
        let mut model = Model::empty(4);
        model.cpu_history.push(990);
        app.apply_model(model);
        assert_eq!(app.alerts().len(), 0);
    }

    #[test]
    fn nice_dialog_opens_at_the_current_value_and_confirms() {
        let mut app = app_with(vec![row(7, 0, 5, "victim")]);
        app.handle_key(Key::Down);
        assert_eq!(app.handle_key(Key::Char('n')), None);
        assert!(app.dialog.is_some());

        app.handle_key(Key::PageDown);
        let command = app.handle_key(Key::Enter);

        assert_eq!(
            command,
            Some(Command::SetNice {
                id: ProcessId::new(Pid::new(7), borbatop_core::StartTime::new(1)),
                nice: borbatop_core::Nice::clamped(5),
            })
        );
        assert!(app.dialog.is_none());
    }

    #[test]
    fn affinity_dialog_needs_cores_and_returns_the_selection() {
        let mut app = app_with(vec![row(7, 0, 5, "victim")]);
        app.handle_key(Key::Char('c'));
        assert!(app.dialog.is_none(), "no cores known yet");

        let mut model = app.model.clone();
        model.cores = vec![Permille::new(0); 4];
        app.apply_model(model);
        app.handle_key(Key::Char('c'));
        assert!(app.dialog.is_some());

        app.handle_key(Key::Char(' ')); // drop CPU 0
        let command = app.handle_key(Key::Enter);
        assert_eq!(
            command,
            Some(Command::SetAffinity {
                id: ProcessId::new(Pid::new(7), borbatop_core::StartTime::new(1)),
                cpus: vec![1, 2, 3],
            })
        );
    }

    #[test]
    fn service_dialog_opens_only_on_the_services_tab() {
        let mut app = app_with(vec![row(7, 0, 5, "victim")]);
        let mut model = Model::empty(4);
        model.services = Ok(parse_systemctl_units("a.service loaded active running A\n"));
        app.apply_model(model);

        app.handle_key(Key::Char('a'));
        assert!(app.dialog.is_none(), "processes tab ignores 'a'");

        app.handle_key(Key::Char('2'));
        app.handle_key(Key::Char('a'));
        assert!(app.dialog.is_some());
        assert_eq!(
            app.handle_key(Key::Enter),
            Some(Command::ControlUnit {
                unit: "a.service".to_owned(),
                action: crate::input::ServiceAction::Restart,
            })
        );
    }

    #[test]
    fn pause_freezes_the_model_and_resume_accepts_it_again() {
        let mut app = app_with(vec![row(1, 0, 5, "a")]);
        app.handle_key(Key::Char(' '));
        let mut newer = Model::empty(4);
        newer.processes = vec![row(2, 0, 5, "b")];
        app.apply_model(newer.clone());
        assert_eq!(pids(&app), [1]);
        app.handle_key(Key::Char(' '));
        app.apply_model(newer);
        assert_eq!(pids(&app), [2]);
    }

    #[test]
    fn opening_the_pacman_tab_requests_the_report_once() {
        let mut app = app_with(vec![]);
        assert_eq!(app.handle_key(Key::Char('3')), Some(Command::RefreshPacman));
        assert!(app.is_pacman_loading());
        assert_eq!(app.handle_key(Key::Char('R')), None);
        app.handle_key(Key::Char('1'));
        assert_eq!(app.handle_key(Key::Char('3')), None);
        assert_eq!(app.tab(), Tab::Pacman);
    }

    #[test]
    fn tab_keys_cycle_and_digits_jump() {
        let mut app = app_with(vec![]);
        app.handle_key(Key::BackTab);
        assert_eq!(app.tab(), Tab::Io);
        app.handle_key(Key::Tab);
        assert_eq!(app.tab(), Tab::Processes);
        app.handle_key(Key::Char('2'));
        assert_eq!(app.tab(), Tab::Services);
        app.handle_key(Key::Char('9'));
        assert_eq!(app.tab(), Tab::Services);
    }

    #[test]
    fn services_list_failed_units_first_and_offer_logs() {
        let mut app = app_with(vec![]);
        let list = parse_systemctl_units(
            "b.service loaded active running B\nz.service loaded failed failed Z\na.service loaded inactive dead A\n",
        );
        let mut model = Model::empty(4);
        model.services = Ok(list);
        app.apply_model(model);
        app.handle_key(Key::Char('2'));

        let names: Vec<&str> = app.visible_units().iter().map(|unit| unit.name()).collect();
        assert_eq!(names, ["z.service", "a.service", "b.service"]);

        assert_eq!(
            app.handle_key(Key::Char('l')),
            Some(Command::LoadLogs {
                unit: "z.service".to_owned()
            })
        );
        app.handle_key(Key::Down);
        assert_eq!(app.selected_unit.as_deref(), Some("a.service"));
        app.set_logs("z.service".to_owned(), Ok("line 1\nline 2".to_owned()));
        assert_eq!(app.logs.as_ref().unwrap().lines.len(), 2);
        app.handle_key(Key::Escape);
        assert!(app.logs.is_none());
    }

    #[test]
    fn messages_fade_after_a_few_ticks() {
        let mut app = app_with(vec![]);
        app.notify("done", MessageKind::Info);
        for _ in 0..3 {
            app.tick();
        }
        assert!(app.message.is_some());
        app.tick();
        assert!(app.message.is_none());
    }

    #[test]
    fn debug_output_hides_the_command_line_of_a_row() {
        let printed = format!("{:?}", row(7, 0, 5, "tool --token=s3cr3t"));
        assert!(!printed.contains("s3cr3t"));
        assert!(printed.contains("<redacted>"));
    }

    #[test]
    fn unrelated_keys_do_nothing() {
        let mut app = app_with(vec![]);
        assert_eq!(app.handle_key(Key::Enter), None);
        assert_eq!(app.handle_key(Key::Backspace), None);
        assert_eq!(app.handle_key(Key::Char('%')), None);
    }
}
