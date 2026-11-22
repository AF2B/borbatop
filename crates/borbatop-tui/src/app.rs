//! Screen state and key handling: no terminal, no drawing, fully testable.

use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use borbatop_core::{ActiveState, Pid, ProcessId, UnitRow, order_as_tree};

use crate::alerts::{Alert, AlertThresholds, evaluate};
use crate::dialog::{Dialog, DialogOutcome};
use crate::input::{
    Command, DoctorRequest, ExportContent, ExportOrder, ExportRequest, Key, Seek, SortKey, Tab,
    UpdatePlan,
};
use crate::model::{
    ContainerView, ContainersView, Model, PacmanView, ProcessRow, Report, ReportLine, ReportTone,
};
use crate::mouse::{HitMap, Mouse};
use crate::theme::ThemeKind;
use crate::widgets::safe;

const MESSAGE_TTL_TICKS: u8 = 4;
const PAGE_ROWS: isize = 10;
const DEFAULT_HISTORY_LEN: usize = 60;
const FIRST_TAB_DIGIT: u32 = 1;
const WHEEL_ROWS: usize = 3;

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
    Loaded(Box<PacmanView>),
}

/// Progress of a report that is loaded on demand; a tab with no entry was never requested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Fetch<T> {
    Loading,
    Loaded(T),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Message {
    pub(crate) text: String,
    pub(crate) kind: MessageKind,
    ttl: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LogView {
    pub(crate) title: String,
    pub(crate) lines: Vec<String>,
    pub(crate) failed: bool,
    /// Lines scrolled up from the newest one; 0 shows the end of the journal.
    pub(crate) scroll: usize,
}

/// Everything the filtered, ordered lists depend on; equal keys give equal lists.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ListKey {
    model_version: u64,
    filter: String,
    sort: SortKey,
    descending: bool,
    layout: ProcessLayout,
}

/// The lists last computed for a key, so a keypress or a redraw does not refilter everything.
#[derive(Debug, Default)]
struct ListCache {
    key: Option<ListKey>,
    processes: Vec<VisibleRow>,
    /// Positions in the model's unit list, in display order.
    units: Vec<usize>,
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
    /// Position within a recording being replayed: `(index, total)`, counted from zero.
    pub(crate) replay: Option<(usize, usize)>,
    pub(crate) alerts: Vec<Alert>,
    pub(crate) pacman: PacmanState,
    pub(crate) reports: HashMap<Tab, Fetch<Report>>,
    /// The Containers tab; `None` until it was first opened.
    pub(crate) containers: Option<Fetch<ContainersView>>,
    /// `(runtime, id)` of the selected container.
    pub(crate) selected_container: Option<(String, String)>,
    pub(crate) scroll: usize,
    /// Largest useful scroll offset, learned while drawing.
    pub(crate) max_scroll: Cell<usize>,
    /// What the last frame drew where, so clicks can be resolved.
    pub(crate) hits: RefCell<HitMap>,
    /// Bumped whenever the shown model is replaced.
    model_version: u64,
    lists: RefCell<ListCache>,
    #[cfg(test)]
    pub(crate) list_computations: Cell<usize>,
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
            replay: None,
            alerts: Vec::new(),
            pacman: PacmanState::NotRequested,
            reports: HashMap::new(),
            containers: None,
            selected_container: None,
            scroll: 0,
            max_scroll: Cell::new(0),
            hits: RefCell::new(HitMap::default()),
            model_version: 0,
            lists: RefCell::new(ListCache::default()),
            #[cfg(test)]
            list_computations: Cell::new(0),
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
            self.model_version = self.model_version.wrapping_add(1);
            self.refresh_alerts();
        }
        self.ensure_selection();
    }

    /// Shows `model` even while paused: stepping through a recording is deliberate.
    ///
    /// # Arguments
    ///
    /// * `model` - the sample to show
    pub fn show_model(&mut self, model: Model) {
        self.model = model;
        self.model_version = self.model_version.wrapping_add(1);
        self.refresh_alerts();
        self.ensure_selection();
    }

    /// Marks the screen as replaying a recording and says where in it the shown sample sits.
    ///
    /// # Arguments
    ///
    /// * `index` - zero-based position of the shown sample
    /// * `total` - number of samples in the recording
    pub fn set_replay(&mut self, index: usize, total: usize) {
        self.replay = Some((index, total));
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
        self.pacman = PacmanState::Loaded(Box::new(view));
    }

    /// Opens the journal overlay for `unit`.
    ///
    /// # Arguments
    ///
    /// * `unit` - unit the text belongs to
    /// * `result` - journal text, or why it could not be read
    pub fn set_logs(&mut self, unit: &str, result: Result<String, String>) {
        self.set_overlay(format!("JOURNAL · {unit}"), result);
    }

    /// Opens the text overlay with a title and a body, or the reason there is none.
    ///
    /// # Arguments
    ///
    /// * `title` - what the text is about
    /// * `result` - the text, or why it could not be produced
    pub fn set_overlay(&mut self, title: String, result: Result<String, String>) {
        let (lines, failed) = match result {
            Ok(text) => (text.lines().map(safe).collect(), false),
            Err(reason) => (vec![safe(&reason)], true),
        };
        self.logs = Some(LogView {
            title,
            lines,
            failed,
            scroll: 0,
        });
    }

    /// Stores a freshly built report of an on-demand tab; a failure becomes one red line.
    ///
    /// # Arguments
    ///
    /// * `tab` - the tab the report belongs to
    /// * `result` - the report, or why it could not be built
    pub fn set_report(&mut self, tab: Tab, result: Result<Report, String>) {
        let report = result.unwrap_or_else(|reason| Report {
            lines: vec![ReportLine::new(reason, ReportTone::Bad)],
        });
        self.reports.insert(tab, Fetch::Loaded(report));
    }

    /// Stores the freshly listed containers, or why they could not be listed.
    ///
    /// # Arguments
    ///
    /// * `result` - the containers, or the failure message
    pub fn set_containers(&mut self, result: Result<ContainersView, String>) {
        let view = result.unwrap_or_else(|reason| ContainersView {
            problems: vec![reason],
            ..ContainersView::default()
        });
        self.containers = Some(Fetch::Loaded(view));
        self.ensure_container_selection();
    }

    /// Whether the container list is being built.
    #[must_use]
    pub fn is_containers_loading(&self) -> bool {
        matches!(self.containers, Some(Fetch::Loading))
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

    /// Whether the report of `tab` is being built.
    #[must_use]
    pub fn is_report_loading(&self, tab: Tab) -> bool {
        matches!(self.reports.get(&tab), Some(Fetch::Loading))
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
        self.ensure_container_selection();
    }

    /// The containers that are listed, in display order.
    pub(crate) fn listed_containers(&self) -> &[ContainerView] {
        match &self.containers {
            Some(Fetch::Loaded(view)) => &view.containers,
            Some(Fetch::Loading) | None => &[],
        }
    }

    fn container_key(container: &ContainerView) -> (String, String) {
        (container.runtime.clone(), container.id.clone())
    }

    /// Keeps a container selected whenever there is one to select.
    fn ensure_container_selection(&mut self) {
        let containers = self.listed_containers();
        let still_listed = containers
            .iter()
            .any(|container| Some(Self::container_key(container)) == self.selected_container);
        if !still_listed {
            self.selected_container = containers.first().map(Self::container_key);
        }
    }

    fn selected_container_view(&self) -> Option<&ContainerView> {
        if self.tab != Tab::Containers {
            return None;
        }
        self.listed_containers()
            .iter()
            .find(|container| Some(Self::container_key(container)) == self.selected_container)
    }

    fn move_container_selection(&mut self, delta: isize) {
        let containers = self.listed_containers();
        let Some(last) = containers.len().checked_sub(1) else {
            return;
        };
        let current = containers
            .iter()
            .position(|container| Some(Self::container_key(container)) == self.selected_container)
            .unwrap_or(0);
        let target = current.saturating_add_signed(delta).min(last);
        self.selected_container = containers.get(target).map(Self::container_key);
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
            Key::Left => Some(Command::Seek(Seek::Backward)),
            Key::Right => Some(Command::Seek(Seek::Forward)),
            Key::Enter => self.request_detail(),
            Key::Backspace => None,
        }
    }

    /// Applies a mouse action and says what, if anything, the program must do.
    ///
    /// Clicks are resolved against what the last frame drew: a tab, a column title or a row.
    /// Dialogs and the filter prompt take no mouse input.
    ///
    /// # Arguments
    ///
    /// * `mouse` - the mouse action
    pub fn handle_mouse(&mut self, mouse: Mouse) -> Option<Command> {
        let command = self.dispatch_mouse(mouse);
        self.ensure_selection();
        command
    }

    fn dispatch_mouse(&mut self, mouse: Mouse) -> Option<Command> {
        if self.dialog.is_some() || self.input == InputMode::Filtering {
            return None;
        }
        match mouse {
            Mouse::WheelUp => {
                self.wheel(Nav::Up, Key::Up);
                None
            }
            Mouse::WheelDown => {
                self.wheel(Nav::Down, Key::Down);
                None
            }
            Mouse::Click { column, row } => self.click(column, row),
        }
    }

    /// Scrolls the journal when it is open, otherwise moves the selection, a few rows.
    fn wheel(&mut self, nav: Nav, key: Key) {
        for _ in 0..WHEEL_ROWS {
            if self.logs.is_some() {
                self.handle_logs_key(key);
            } else {
                self.navigate(nav);
            }
        }
    }

    fn click(&mut self, column: u16, row: u16) -> Option<Command> {
        if self.logs.is_some() {
            return None;
        }
        let hits = self.hits.borrow().clone();
        if let Some(tab) = hits.tab_at(column, row) {
            return self.select_tab(tab);
        }
        if let Some(key) = hits.sort_at(column, row) {
            self.sort_by(key);
        } else if let Some(position) = hits.list_position_at(row) {
            self.select_position(position);
        }
        None
    }

    /// Sorts by `key`; a second request for the same column reverses the order.
    fn sort_by(&mut self, key: SortKey) {
        if self.sort == key {
            self.descending = !self.descending;
        } else {
            self.sort = key;
            self.descending = key.descending_by_default();
        }
        self.jump_to_top();
    }

    /// Selects the first row of the new order, so the effect of a sort is always visible: the
    /// view follows the selection, and a cursor left in the middle would hide the change.
    fn jump_to_top(&mut self) {
        if self.tab != Tab::Processes {
            return;
        }
        let first = self
            .visible_processes()
            .first()
            .and_then(|row| self.model.processes.get(row.index))
            .map(|process| process.pid);
        if first.is_some() {
            self.selected_pid = first;
        }
    }

    /// Selects the row at `position` of the list the current tab shows.
    fn select_position(&mut self, position: usize) {
        match self.tab {
            Tab::Processes => {
                let rows = self.visible_processes();
                self.selected_pid = rows
                    .get(position)
                    .and_then(|row| self.model.processes.get(row.index))
                    .map(|process| process.pid);
            }
            Tab::Services => {
                let name = self
                    .visible_units()
                    .get(position)
                    .map(|unit| unit.name().to_owned());
                if name.is_some() {
                    self.selected_unit = name;
                }
            }
            Tab::Containers => {
                let key = self
                    .listed_containers()
                    .get(position)
                    .map(Self::container_key);
                if key.is_some() {
                    self.selected_container = key;
                }
            }
            Tab::Pacman | Tab::Io | Tab::Connections | Tab::Boot | Tab::Logs | Tab::Gpu => {}
        }
    }

    fn tab_offset(&self, delta: isize) -> isize {
        isize::try_from(self.tab.index())
            .unwrap_or(0)
            .saturating_add(delta)
    }

    fn handle_char(&mut self, character: char) -> Option<Command> {
        match character {
            'D' if self.tab == Tab::Pacman => self.request_doctor(),
            's' if self.tab == Tab::Pacman => self.request_snapshot(),
            'p' if self.tab == Tab::Pacman => self.request_config_merge(),
            'd' if self.tab == Tab::Pacman => self.request_pacnew_diffs(),
            'x' if self.tab == Tab::Pacman => self.request_service_restart(),
            'q' => Some(Command::Quit),
            'j' => self.navigate(Nav::Down),
            'k' => self.navigate(Nav::Up),
            'g' => self.navigate(Nav::Top),
            'G' => self.navigate(Nav::Bottom),
            's' => {
                self.sort = self.sort.next();
                self.descending = self.sort.descending_by_default();
                self.jump_to_top();
                None
            }
            'r' => {
                self.descending = !self.descending;
                self.jump_to_top();
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
            'R' => self.request_reload(),
            'l' => self.request_logs(),
            'e' => self.request_export(),
            'u' => self.open_update_dialog(),
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
        if tab.is_report() && !self.reports.contains_key(&tab) {
            return self.request_report(tab);
        }
        if tab == Tab::Containers && self.containers.is_none() {
            return self.request_containers();
        }
        None
    }

    /// Rebuilds the report of the tab being shown, if it has one.
    fn request_reload(&mut self) -> Option<Command> {
        if self.tab.is_report() {
            self.request_report(self.tab)
        } else if self.tab == Tab::Containers {
            self.request_containers()
        } else {
            self.request_pacman()
        }
    }

    fn request_containers(&mut self) -> Option<Command> {
        if self.is_containers_loading() {
            return None;
        }
        self.containers = Some(Fetch::Loading);
        Some(Command::LoadContainers)
    }

    fn request_report(&mut self, tab: Tab) -> Option<Command> {
        if self.is_report_loading(tab) {
            return None;
        }
        self.reports.insert(tab, Fetch::Loading);
        Some(Command::LoadReport(tab))
    }

    fn request_pacman(&mut self) -> Option<Command> {
        if self.tab != Tab::Pacman || self.is_pacman_loading() {
            return None;
        }
        self.pacman = PacmanState::Loading;
        Some(Command::RefreshPacman)
    }

    /// Asks to write the tab being shown to a file, exactly as it is filtered and ordered.
    ///
    /// A tab whose report has not arrived yet is not exported: the person is told to wait.
    fn request_export(&mut self) -> Option<Command> {
        let content = match self.tab {
            Tab::Processes => ExportContent::Processes(
                self.visible_processes()
                    .iter()
                    .filter_map(|row| self.model.processes.get(row.index))
                    .map(|process| process.pid)
                    .collect(),
            ),
            Tab::Services => match &self.model.services {
                Ok(_) => ExportContent::Services(
                    self.visible_units()
                        .iter()
                        .map(|unit| unit.name().to_owned())
                        .collect(),
                ),
                Err(reason) => {
                    let text = format!("nothing to export: {reason}");
                    self.notify(text, MessageKind::Error);
                    return None;
                }
            },
            Tab::Io => ExportContent::Io,
            Tab::Pacman => match &self.pacman {
                PacmanState::Loaded(view) => ExportContent::Pacman(view.clone()),
                PacmanState::NotRequested | PacmanState::Loading => {
                    return self.not_ready_to_export();
                }
            },
            Tab::Containers => match &self.containers {
                Some(Fetch::Loaded(view)) => ExportContent::Containers(Box::new(view.clone())),
                Some(Fetch::Loading) | None => return self.not_ready_to_export(),
            },
            Tab::Connections | Tab::Boot | Tab::Logs | Tab::Gpu => {
                match self.reports.get(&self.tab) {
                    Some(Fetch::Loaded(report)) => ExportContent::Report(report.clone()),
                    Some(Fetch::Loading) | None => return self.not_ready_to_export(),
                }
            }
        };
        let order = (self.tab == Tab::Processes).then_some(ExportOrder {
            column: self.sort,
            descending: self.descending,
            tree: self.layout == ProcessLayout::Tree,
        });
        Some(Command::Export(Box::new(ExportRequest {
            tab: self.tab,
            filter: self.filter.trim().to_owned(),
            order,
            content,
        })))
    }

    fn not_ready_to_export(&mut self) -> Option<Command> {
        self.notify(
            "nothing to export yet: wait for this tab to finish loading",
            MessageKind::Error,
        );
        None
    }

    /// Asks for everything known about the selected process.
    fn request_detail(&self) -> Option<Command> {
        let process = self.selected_process()?;
        let program = process
            .command
            .split_whitespace()
            .next()
            .and_then(|path| path.rsplit('/').next())
            .unwrap_or_default();
        Some(Command::LoadDetail {
            id: ProcessId::new(process.pid, process.start_time),
            label: format!("PID {} · {}", process.pid, program),
        })
    }

    fn request_logs(&mut self) -> Option<Command> {
        match self.tab {
            Tab::Services => self
                .selected_unit
                .clone()
                .map(|unit| Command::LoadLogs { unit }),
            Tab::Containers => {
                self.selected_container_view()
                    .map(|container| Command::LoadContainerLogs {
                        runtime: container.runtime.clone(),
                        id: container.id.clone(),
                        name: container.name.clone(),
                    })
            }
            Tab::Processes
            | Tab::Pacman
            | Tab::Io
            | Tab::Connections
            | Tab::Boot
            | Tab::Logs
            | Tab::Gpu => None,
        }
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

    /// The Pacman report, when it is ready; otherwise tells the person to wait.
    fn ready_pacman(&mut self) -> Option<PacmanView> {
        if let PacmanState::Loaded(view) = &self.pacman {
            return Some((**view).clone());
        }
        self.notify(
            "wait for the Pacman report to finish loading",
            MessageKind::Error,
        );
        None
    }

    /// Systemd services that run with a library that was replaced on disk, from the report and
    /// the processes on screen.
    fn stale_service_units(&self) -> Vec<String> {
        let PacmanState::Loaded(view) = &self.pacman else {
            return Vec::new();
        };
        let mut units: Vec<String> = view
            .stale
            .iter()
            .filter_map(|stale| {
                self.model
                    .processes
                    .iter()
                    .find(|process| process.pid == stale.pid)
            })
            .map(|process| process.unit.clone())
            .filter(|unit| unit.ends_with(".service"))
            .collect();
        units.sort();
        units.dedup();
        units
    }

    fn request_doctor(&mut self) -> Option<Command> {
        let pacman = self.ready_pacman()?;
        Some(Command::LoadDoctor(Box::new(DoctorRequest {
            pacman,
            stale_units: self.stale_service_units(),
        })))
    }

    fn request_snapshot(&mut self) -> Option<Command> {
        let view = self.ready_pacman()?;
        if let Some(tool) = view.snapshot_tool {
            return Some(Command::RunUpdate(UpdatePlan::Snapshot { tool }));
        }
        self.notify(
            "no snapshot tool found: install snapper or timeshift",
            MessageKind::Error,
        );
        None
    }

    /// The `.pacnew` files the report found, or why there are none to show.
    fn pending_config_files(&mut self) -> Option<Vec<String>> {
        let view = self.ready_pacman()?;
        match view.pending_config {
            Ok(files) if !files.is_empty() => Some(files),
            Ok(_) => {
                self.notify("no .pacnew files to review", MessageKind::Info);
                None
            }
            Err(reason) => {
                self.notify(
                    format!("cannot list .pacnew files: {reason}"),
                    MessageKind::Error,
                );
                None
            }
        }
    }

    fn request_config_merge(&mut self) -> Option<Command> {
        self.pending_config_files()
            .map(|_files| Command::RunUpdate(UpdatePlan::MergeConfigs))
    }

    fn request_pacnew_diffs(&mut self) -> Option<Command> {
        let files = self.pending_config_files()?;
        let pacnew: Vec<String> = files
            .into_iter()
            .filter(|file| file.ends_with(".pacnew"))
            .collect();
        if pacnew.is_empty() {
            self.notify(
                "only .pacsave files are left: nothing to compare",
                MessageKind::Info,
            );
            return None;
        }
        Some(Command::LoadPacnewDiffs(pacnew))
    }

    fn request_service_restart(&mut self) -> Option<Command> {
        self.ready_pacman()?;
        let units = self.stale_service_units();
        if units.is_empty() {
            self.notify(
                "no service is running with a replaced library",
                MessageKind::Info,
            );
            return None;
        }
        Some(Command::RunUpdate(UpdatePlan::RestartUnits { units }))
    }

    /// Opens the update picker for what the Pacman report found.
    fn open_update_dialog(&mut self) -> Option<Command> {
        if self.tab != Tab::Pacman {
            return None;
        }
        let PacmanState::Loaded(view) = &self.pacman else {
            self.notify(
                "wait for the Pacman report to finish loading",
                MessageKind::Error,
            );
            return None;
        };
        if let Some(dialog) = Dialog::update(view) {
            self.dialog = Some(dialog);
            return None;
        }
        let unknown = view.upgrades.is_err() || view.aur.iter().any(|aur| aur.upgrades.is_err());
        if unknown {
            self.notify(
                "cannot tell what is pending: some upgrade lists could not be read",
                MessageKind::Error,
            );
        } else {
            self.notify("nothing to update", MessageKind::Info);
        }
        None
    }

    fn open_service_dialog(&mut self) -> Option<Command> {
        if self.tab == Tab::Containers {
            let container = self.selected_container_view()?;
            self.dialog = Some(Dialog::container(
                &container.runtime,
                &container.id,
                &container.name,
            ));
            return None;
        }
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
            DialogOutcome::Switch(next) => {
                self.dialog = Some(*next);
                None
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
            Tab::Containers => self.move_container_selection(delta),
            Tab::Pacman | Tab::Io | Tab::Connections | Tab::Boot | Tab::Logs | Tab::Gpu => {
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

    /// Recomputes the filtered lists when anything they depend on changed.
    fn ensure_lists(&self) {
        let key = ListKey {
            model_version: self.model_version,
            filter: self.filter.clone(),
            sort: self.sort,
            descending: self.descending,
            layout: self.layout,
        };
        let mut cache = self.lists.borrow_mut();
        if cache.key.as_ref() == Some(&key) {
            return;
        }
        #[cfg(test)]
        self.list_computations
            .set(self.list_computations.get().saturating_add(1));
        cache.processes = self.compute_visible_processes();
        cache.units = self.compute_visible_units();
        cache.key = Some(key);
    }

    /// Processes that pass the filter, in display order (tree order or sorted).
    pub(crate) fn visible_processes(&self) -> Vec<VisibleRow> {
        self.ensure_lists();
        self.lists.borrow().processes.clone()
    }

    fn compute_visible_processes(&self) -> Vec<VisibleRow> {
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
        self.ensure_lists();
        let Ok(list) = &self.model.services else {
            return Vec::new();
        };
        self.lists
            .borrow()
            .units
            .iter()
            .filter_map(|&position| list.units().get(position))
            .collect()
    }

    fn compute_visible_units(&self) -> Vec<usize> {
        let Ok(list) = &self.model.services else {
            return Vec::new();
        };
        let needle = self.filter.trim().to_lowercase();
        let mut units: Vec<(usize, &UnitRow)> = list
            .units()
            .iter()
            .enumerate()
            .filter(|(_, unit)| {
                needle.is_empty()
                    || unit.name().to_lowercase().contains(&needle)
                    || unit.description().to_lowercase().contains(&needle)
            })
            .collect();
        units.sort_by_key(|(_, unit)| (unit.active() != ActiveState::Failed, unit.name()));
        units.into_iter().map(|(position, _)| position).collect()
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
        Key::Char(_)
        | Key::Left
        | Key::Right
        | Key::Enter
        | Key::Escape
        | Key::Backspace
        | Key::Tab
        | Key::BackTab => None,
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

    fn exported(app: &mut App) -> ExportRequest {
        app.handle_key(Key::Char('e'))
            .and_then(|command| match command {
                Command::Export(request) => Some(*request),
                Command::Quit
                | Command::Seek(_)
                | Command::SendSignal { .. }
                | Command::SetNice { .. }
                | Command::SetAffinity { .. }
                | Command::ControlUnit { .. }
                | Command::LoadDetail { .. }
                | Command::LoadReport(_)
                | Command::LoadContainers
                | Command::RunUpdate(_)
                | Command::LoadDoctor(_)
                | Command::LoadPacnewDiffs(_)
                | Command::ControlContainer { .. }
                | Command::LoadContainerLogs { .. }
                | Command::RefreshPacman
                | Command::LoadLogs { .. } => None,
            })
            .unwrap()
    }

    #[test]
    fn export_carries_what_the_person_sees_filter_order_and_rows() {
        let mut app = app_with(vec![
            row(1, 0, 10, "alpha"),
            row(2, 0, 900, "zeta"),
            row(3, 0, 50, "zebra"),
        ]);
        app.handle_key(Key::Char('/'));
        for character in "ze".chars() {
            app.handle_key(Key::Char(character));
        }
        app.handle_key(Key::Enter);

        let request = exported(&mut app);

        assert_eq!(request.tab, Tab::Processes);
        assert_eq!(request.filter, "ze");
        assert_eq!(
            request.order,
            Some(ExportOrder {
                column: SortKey::Cpu,
                descending: true,
                tree: false,
            })
        );
        assert_eq!(
            request.content,
            ExportContent::Processes(vec![Pid::new(2), Pid::new(3)]),
            "only the rows that pass the filter, biggest cpu first"
        );
    }

    #[test]
    fn typing_e_in_the_filter_prompt_is_just_a_letter() {
        let mut app = app_with(vec![row(1, 0, 10, "alpha")]);
        app.handle_key(Key::Char('/'));
        assert_eq!(app.handle_key(Key::Char('e')), None);
        assert_eq!(app.filter, "e");
    }

    #[test]
    fn tabs_that_are_not_ready_refuse_to_export_and_say_why() {
        let mut app = app_with(vec![]);
        app.handle_key(Key::Char('3')); // pacman, still loading
        assert_eq!(app.handle_key(Key::Char('e')), None);
        assert!(app.message.as_ref().unwrap().text.contains("wait"));

        app.handle_key(Key::Char('5')); // connections, still loading
        app.message = None;
        assert_eq!(app.handle_key(Key::Char('e')), None);
        assert_eq!(app.message.as_ref().unwrap().kind, MessageKind::Error);
    }

    #[test]
    fn a_loaded_report_and_the_io_tab_export() {
        let mut app = app_with(vec![]);
        app.handle_key(Key::Char('7'));
        app.set_report(
            Tab::Logs,
            Ok(Report {
                lines: vec![ReportLine::new("kernel: bad", ReportTone::Bad)],
            }),
        );
        let logs = exported(&mut app);
        assert_eq!(logs.tab, Tab::Logs);
        assert_eq!(logs.order, None);
        assert!(
            matches!(logs.content, ExportContent::Report(ref report) if report.lines.len() == 1)
        );

        app.handle_key(Key::Char('4'));
        assert_eq!(exported(&mut app).content, ExportContent::Io);
    }

    #[test]
    fn services_export_the_filtered_names_and_refuse_when_systemd_failed() {
        let mut app = app_with(vec![]);
        app.handle_key(Key::Char('2'));
        assert_eq!(app.handle_key(Key::Char('e')), None, "no systemd data yet");

        let mut model = Model::empty(4);
        model.services = Ok(parse_systemctl_units(
            "b.service loaded active running B\nz.service loaded failed failed Z\n",
        ));
        app.apply_model(model);
        let request = exported(&mut app);
        assert_eq!(
            request.content,
            ExportContent::Services(vec!["z.service".to_owned(), "b.service".to_owned()]),
            "failed units first, as on screen"
        );
    }

    fn container(runtime: &str, id: &str, name: &str, running: bool) -> ContainerView {
        ContainerView {
            runtime: runtime.to_owned(),
            id: id.to_owned(),
            name: name.to_owned(),
            image: "nginx".to_owned(),
            status: if running { "Up 3 hours" } else { "Exited (0)" }.to_owned(),
            running,
            usage: "3 processes · cpu 0.4% · mem 54.0M".to_owned(),
        }
    }

    fn loaded_containers() -> App {
        let mut app = app_with(vec![]);
        assert_eq!(
            app.handle_key(Key::Char('9')),
            Some(Command::LoadContainers)
        );
        app.set_containers(Ok(ContainersView {
            containers: vec![
                container("podman", "aaa111", "web", true),
                container("docker", "bbb222", "db", false),
            ],
            has_runtime: true,
            ..ContainersView::default()
        }));
        app
    }

    #[test]
    fn the_containers_tab_loads_once_selects_and_acts_on_the_chosen_container() {
        let mut app = loaded_containers();
        assert!(!app.is_containers_loading());
        assert_eq!(app.handle_key(Key::Char('1')), None);
        assert_eq!(
            app.handle_key(Key::Char('9')),
            None,
            "a loaded list is kept"
        );
        assert_eq!(
            app.selected_container,
            Some(("podman".to_owned(), "aaa111".to_owned()))
        );

        app.handle_key(Key::Down);
        assert_eq!(
            app.selected_container,
            Some(("docker".to_owned(), "bbb222".to_owned()))
        );

        assert_eq!(
            app.handle_key(Key::Char('l')),
            Some(Command::LoadContainerLogs {
                runtime: "docker".to_owned(),
                id: "bbb222".to_owned(),
                name: "db".to_owned(),
            })
        );

        assert_eq!(app.handle_key(Key::Char('a')), None);
        assert!(app.dialog.is_some());
        app.handle_key(Key::Down); // stop
        app.handle_key(Key::Down); // start
        assert_eq!(
            app.handle_key(Key::Enter),
            Some(Command::ControlContainer {
                runtime: "docker".to_owned(),
                id: "bbb222".to_owned(),
                action: crate::input::ContainerChoice::Start,
            })
        );
    }

    #[test]
    fn container_selection_survives_a_refresh_and_falls_back_when_it_disappears() {
        let mut app = loaded_containers();
        app.handle_key(Key::Down);
        app.set_containers(Ok(ContainersView {
            containers: vec![
                container("podman", "aaa111", "web", true),
                container("docker", "bbb222", "db", true),
            ],
            has_runtime: true,
            ..ContainersView::default()
        }));
        assert_eq!(
            app.selected_container,
            Some(("docker".to_owned(), "bbb222".to_owned()))
        );

        app.set_containers(Ok(ContainersView {
            containers: vec![container("podman", "aaa111", "web", true)],
            has_runtime: true,
            ..ContainersView::default()
        }));
        assert_eq!(
            app.selected_container,
            Some(("podman".to_owned(), "aaa111".to_owned()))
        );

        assert_eq!(
            app.handle_key(Key::Char('R')),
            Some(Command::LoadContainers)
        );
        assert_eq!(app.handle_key(Key::Char('R')), None, "already loading");
    }

    #[test]
    fn a_runtime_that_failed_is_shown_as_a_problem_and_nothing_is_selectable() {
        let mut app = app_with(vec![]);
        app.handle_key(Key::Char('9'));
        app.set_containers(Err("docker: permission denied".to_owned()));
        assert_eq!(app.selected_container, None);
        assert_eq!(app.handle_key(Key::Char('a')), None);
        assert!(app.dialog.is_none());
        assert_eq!(app.handle_key(Key::Char('l')), None);
    }

    #[test]
    fn export_of_the_containers_tab_waits_for_the_list() {
        let mut app = app_with(vec![]);
        app.handle_key(Key::Char('9'));
        assert_eq!(app.handle_key(Key::Char('e')), None, "still loading");

        let mut loaded = loaded_containers();
        let request = exported(&mut loaded);
        assert_eq!(request.tab, Tab::Containers);
        assert!(
            matches!(request.content, ExportContent::Containers(ref view) if view.containers.len() == 2)
        );
    }

    fn upgrades(text: &str) -> Vec<borbatop_core::Upgrade> {
        borbatop_core::parse_upgrades(text).unwrap()
    }

    fn pacman_view(
        repo: Vec<borbatop_core::Upgrade>,
        aur: Vec<crate::model::AurView>,
    ) -> PacmanView {
        PacmanView {
            upgrades: Ok(repo),
            repo_source: crate::model::RepoSource::Checkupdates,
            snapshot_tool: None,
            aur,
            orphans: Ok(Vec::new()),
            kernel: Err("n/a".to_owned()),
            pending_config: Ok(Vec::new()),
            cache: Err("n/a".to_owned()),
            stale: Vec::new(),
            stale_unreadable: 0,
        }
    }

    fn app_on_pacman(view: PacmanView) -> App {
        let mut app = app_with(vec![]);
        assert_eq!(app.handle_key(Key::Char('3')), Some(Command::RefreshPacman));
        app.set_pacman(view);
        app
    }

    #[test]
    fn updating_waits_for_the_report_and_says_when_there_is_nothing_to_do() {
        let mut app = app_with(vec![]);
        app.handle_key(Key::Char('3'));
        assert_eq!(app.handle_key(Key::Char('u')), None);
        assert!(app.dialog.is_none());
        assert!(app.message.as_ref().unwrap().text.contains("wait"));

        app.set_pacman(pacman_view(Vec::new(), Vec::new()));
        app.message = None;
        app.handle_key(Key::Char('u'));
        assert!(app.dialog.is_none());
        assert_eq!(app.message.as_ref().unwrap().text, "nothing to update");

        app.handle_key(Key::Char('1'));
        app.handle_key(Key::Char('u'));
        assert!(app.dialog.is_none(), "only the Pacman tab updates");
    }

    #[test]
    fn repositories_update_as_a_whole_and_helpers_offer_all_or_a_choice() {
        let view = pacman_view(
            upgrades("linux 1 -> 2\nglibc 1 -> 2\n"),
            vec![
                crate::model::AurView {
                    helper: "yay".to_owned(),
                    upgrades: Ok(upgrades("tool-a 1 -> 2\ntool-b 3 -> 4\n")),
                },
                crate::model::AurView {
                    helper: "paru".to_owned(),
                    upgrades: Err("no network".to_owned()),
                },
            ],
        );
        let mut app = app_on_pacman(view.clone());
        app.handle_key(Key::Char('u'));
        assert!(app.dialog.is_some());
        assert_eq!(
            app.handle_key(Key::Enter),
            Some(Command::RunUpdate(crate::input::UpdatePlan::Repositories))
        );

        let mut app = app_on_pacman(view);
        app.handle_key(Key::Char('u'));
        app.handle_key(Key::Down);
        assert_eq!(
            app.handle_key(Key::Enter),
            Some(Command::RunUpdate(crate::input::UpdatePlan::AurAll {
                helper: "yay".to_owned()
            }))
        );
    }

    #[test]
    fn choosing_aur_packages_needs_at_least_one_and_returns_exactly_the_chosen() {
        let view = pacman_view(
            Vec::new(),
            vec![crate::model::AurView {
                helper: "yay".to_owned(),
                upgrades: Ok(upgrades("tool-a 1 -> 2\ntool-b 3 -> 4\ntool-c 5 -> 6\n")),
            }],
        );
        let mut app = app_on_pacman(view);
        app.handle_key(Key::Char('u'));
        app.handle_key(Key::Down); // "choose packages…"
        assert_eq!(app.handle_key(Key::Enter), None, "opens the picker");
        assert!(app.dialog.is_some());

        assert_eq!(
            app.handle_key(Key::Enter),
            None,
            "nothing chosen yet: stays open"
        );
        assert!(app.dialog.is_some());

        app.handle_key(Key::Char(' ')); // tool-a
        app.handle_key(Key::Down);
        app.handle_key(Key::Down);
        app.handle_key(Key::Char(' ')); // tool-c
        assert_eq!(
            app.handle_key(Key::Enter),
            Some(Command::RunUpdate(crate::input::UpdatePlan::AurPackages {
                helper: "yay".to_owned(),
                packages: vec!["tool-a".to_owned(), "tool-c".to_owned()],
            }))
        );
        assert!(app.dialog.is_none());
    }

    fn service_row(pid: u32, unit: &str) -> ProcessRow {
        let mut process = row(pid, 0, 1, "svc");
        process.unit = unit.to_owned();
        process
    }

    fn stale(pid: u32) -> crate::model::StaleView {
        crate::model::StaleView {
            pid: Pid::new(pid),
            comm: "svc".to_owned(),
            library: "libssl.so.3".to_owned(),
        }
    }

    fn doctor_app(view: PacmanView, rows: Vec<ProcessRow>) -> App {
        let mut app = app_with(rows);
        app.handle_key(Key::Char('3'));
        app.set_pacman(view);
        app
    }

    #[test]
    fn the_doctor_needs_the_report_and_carries_the_services_to_restart() {
        let mut app = app_with(vec![]);
        app.handle_key(Key::Char('3'));
        assert_eq!(app.handle_key(Key::Char('D')), None);
        assert!(app.message.as_ref().unwrap().text.contains("wait"));

        let mut view = pacman_view(Vec::new(), Vec::new());
        view.stale = vec![stale(10), stale(11), stale(12), stale(99)];
        let mut app = doctor_app(
            view.clone(),
            vec![
                service_row(10, "sshd.service"),
                service_row(11, "sshd.service"),
                service_row(12, "app-firefox-3.scope"),
            ],
        );
        let command = app.handle_key(Key::Char('D'));
        assert_eq!(
            command,
            Some(Command::LoadDoctor(Box::new(DoctorRequest {
                pacman: view,
                stale_units: vec!["sshd.service".to_owned()],
            }))),
            "scopes and unknown pids are left out, duplicates collapse"
        );
    }

    #[test]
    fn snapshot_needs_an_installed_tool() {
        let mut app = doctor_app(pacman_view(Vec::new(), Vec::new()), vec![]);
        assert_eq!(app.handle_key(Key::Char('s')), None);
        assert!(
            app.message
                .as_ref()
                .unwrap()
                .text
                .contains("snapper or timeshift")
        );

        let mut view = pacman_view(Vec::new(), Vec::new());
        view.snapshot_tool = Some("snapper".to_owned());
        let mut app = doctor_app(view, vec![]);
        assert_eq!(
            app.handle_key(Key::Char('s')),
            Some(Command::RunUpdate(UpdatePlan::Snapshot {
                tool: "snapper".to_owned()
            }))
        );
    }

    #[test]
    fn config_merge_and_diffs_need_pacnew_files() {
        let mut app = doctor_app(pacman_view(Vec::new(), Vec::new()), vec![]);
        assert_eq!(app.handle_key(Key::Char('p')), None);
        assert_eq!(app.handle_key(Key::Char('d')), None);
        assert!(app.message.as_ref().unwrap().text.contains("no .pacnew"));

        let mut view = pacman_view(Vec::new(), Vec::new());
        view.pending_config = Ok(vec![
            "/etc/pacman.conf.pacnew".to_owned(),
            "/etc/old.conf.pacsave".to_owned(),
        ]);
        let mut app = doctor_app(view, vec![]);
        assert_eq!(
            app.handle_key(Key::Char('p')),
            Some(Command::RunUpdate(UpdatePlan::MergeConfigs))
        );
        assert_eq!(
            app.handle_key(Key::Char('d')),
            Some(Command::LoadPacnewDiffs(vec![
                "/etc/pacman.conf.pacnew".to_owned()
            ])),
            ".pacsave files have nothing to compare"
        );
    }

    #[test]
    fn restarting_services_needs_at_least_one_stale_service() {
        let mut app = doctor_app(pacman_view(Vec::new(), Vec::new()), vec![]);
        assert_eq!(app.handle_key(Key::Char('x')), None);
        assert!(app.message.as_ref().unwrap().text.contains("no service"));

        let mut view = pacman_view(Vec::new(), Vec::new());
        view.stale = vec![stale(10)];
        let mut app = doctor_app(view, vec![service_row(10, "cups.service")]);
        assert_eq!(
            app.handle_key(Key::Char('x')),
            Some(Command::RunUpdate(UpdatePlan::RestartUnits {
                units: vec!["cups.service".to_owned()]
            }))
        );
    }

    #[test]
    fn the_doctor_keys_are_only_special_on_the_pacman_tab() {
        let mut app = app_with(vec![row(1, 0, 10, "a"), row(2, 0, 900, "b")]);
        assert_eq!(app.handle_key(Key::Char('x')), None);
        assert_eq!(app.handle_key(Key::Char('D')), None);
        app.handle_key(Key::Char('s'));
        assert_eq!(
            app.sort,
            SortKey::Memory,
            "s still sorts on the Processes tab"
        );
    }

    #[test]
    fn the_update_dialog_offers_a_snapshot_first_when_a_tool_exists() {
        let mut view = pacman_view(upgrades("linux 1 -> 2\n"), Vec::new());
        view.snapshot_tool = Some("timeshift".to_owned());
        let mut app = app_on_pacman(view);
        app.handle_key(Key::Char('u'));
        assert_eq!(
            app.handle_key(Key::Enter),
            Some(Command::RunUpdate(UpdatePlan::RepositoriesAfterSnapshot {
                tool: "timeshift".to_owned()
            })),
            "the safer option comes first"
        );

        let mut app = app_on_pacman(pacman_view(upgrades("linux 1 -> 2\n"), Vec::new()));
        app.handle_key(Key::Char('u'));
        assert_eq!(
            app.handle_key(Key::Enter),
            Some(Command::RunUpdate(UpdatePlan::Repositories))
        );
    }

    #[test]
    fn unreadable_upgrade_lists_are_not_reported_as_nothing_to_update() {
        let mut view = pacman_view(Vec::new(), Vec::new());
        view.upgrades = Err("no network".to_owned());
        let mut app = app_on_pacman(view);
        app.handle_key(Key::Char('u'));
        assert!(app.dialog.is_none());
        let message = app.message.as_ref().unwrap();
        assert_eq!(message.kind, MessageKind::Error);
        assert!(message.text.contains("cannot tell"), "{}", message.text);
    }

    #[test]
    fn sort_keys_on_other_tabs_leave_the_process_cursor_alone() {
        let mut app = app_with(vec![
            row(1, 0, 10, "a"),
            row(2, 0, 900, "b"),
            row(3, 0, 50, "c"),
        ]);
        app.handle_key(Key::Down);
        let before = app.selected_pid;
        app.handle_key(Key::Char('2'));
        app.handle_key(Key::Char('r'));
        app.handle_key(Key::Char('s'));
        app.handle_key(Key::Char('1'));
        assert_eq!(app.selected_pid, before);
    }

    #[test]
    fn changing_the_order_jumps_to_the_first_row_so_the_effect_is_visible() {
        let rows: Vec<ProcessRow> = (1..=30)
            .map(|number| row(number, 0, u64::from(number), "p"))
            .collect();
        let mut app = app_with(rows);
        for _ in 0..15 {
            app.handle_key(Key::Down);
        }
        assert_eq!(app.selected_pid, Some(Pid::new(15)), "cursor in the middle");

        app.handle_key(Key::Char('r'));
        assert_eq!(
            app.selected_pid,
            Some(Pid::new(1)),
            "reverse: first of the new order"
        );

        app.handle_key(Key::Char('G'));
        app.handle_key(Key::Char('s'));
        let top = app.visible_processes().first().map(|row| row.index);
        assert_eq!(
            app.selected_pid,
            top.and_then(|index| app.model.processes.get(index))
                .map(|p| p.pid),
            "next column: first of the new order"
        );
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
    fn arrow_keys_ask_to_step_through_a_recording() {
        let mut app = app_with(vec![row(1, 0, 5, "a")]);
        assert_eq!(
            app.handle_key(Key::Left),
            Some(Command::Seek(Seek::Backward))
        );
        assert_eq!(
            app.handle_key(Key::Right),
            Some(Command::Seek(Seek::Forward))
        );

        app.handle_key(Key::Char('K'));
        assert_eq!(
            app.handle_key(Key::Left),
            None,
            "a dialog keeps the arrows to itself"
        );
    }

    #[test]
    fn show_model_ignores_the_pause_that_apply_model_respects() {
        let mut app = app_with(vec![row(1, 0, 5, "a")]);
        app.handle_key(Key::Char(' '));
        assert!(app.is_paused());

        let mut next = Model::empty(4);
        next.processes = vec![row(2, 0, 5, "b")];
        app.apply_model(next.clone());
        assert_eq!(app.model().processes.first().map(|p| p.pid.get()), Some(1));

        app.show_model(next);
        assert_eq!(app.model().processes.first().map(|p| p.pid.get()), Some(2));
        assert_eq!(
            app.selected_pid,
            Some(Pid::new(2)),
            "the selection follows the new sample"
        );
    }

    #[test]
    fn report_tabs_request_once_and_reload_on_demand() {
        let mut app = app_with(vec![]);
        assert_eq!(
            app.handle_key(Key::Char('5')),
            Some(Command::LoadReport(Tab::Connections))
        );
        assert!(app.is_report_loading(Tab::Connections));
        assert_eq!(app.handle_key(Key::Char('R')), None, "already loading");

        app.set_report(Tab::Connections, Ok(Report::default()));
        assert!(!app.is_report_loading(Tab::Connections));
        app.handle_key(Key::Char('1'));
        assert_eq!(
            app.handle_key(Key::Char('5')),
            None,
            "a loaded report is kept"
        );
        assert_eq!(
            app.handle_key(Key::Char('R')),
            Some(Command::LoadReport(Tab::Connections))
        );
    }

    #[test]
    fn a_failed_report_becomes_one_red_line() {
        let mut app = app_with(vec![]);
        app.set_report(Tab::Gpu, Err("no card".to_owned()));
        assert_eq!(
            app.reports.get(&Tab::Gpu),
            Some(&Fetch::Loaded(Report {
                lines: vec![ReportLine::new("no card", ReportTone::Bad)],
            }))
        );
    }

    #[test]
    fn enter_asks_for_the_details_of_the_selected_process() {
        let mut app = app_with(vec![row(7, 0, 5, "/usr/bin/victim --flag")]);
        assert_eq!(
            app.handle_key(Key::Enter),
            Some(Command::LoadDetail {
                id: ProcessId::new(Pid::new(7), borbatop_core::StartTime::new(1)),
                label: "PID 7 · victim".to_owned(),
            })
        );

        app.handle_key(Key::Char('2'));
        assert_eq!(
            app.handle_key(Key::Enter),
            None,
            "only the process tab has details"
        );
    }

    #[test]
    fn lists_are_recomputed_only_when_their_inputs_change() {
        let mut app = app_with(vec![row(1, 0, 10, "alpha"), row(2, 0, 900, "zeta")]);
        let baseline = app.list_computations.get();

        for _ in 0..5 {
            assert_eq!(pids(&app), [2, 1]);
            app.handle_key(Key::Down);
            app.handle_key(Key::Up);
        }
        assert_eq!(app.list_computations.get(), baseline, "moving is free");

        app.handle_key(Key::Char('r'));
        assert_eq!(
            app.list_computations.get(),
            baseline + 1,
            "sorting recomputes"
        );
        assert_eq!(pids(&app), [1, 2]);

        app.handle_key(Key::Char('/'));
        for character in "zeta".chars() {
            app.handle_key(Key::Char(character));
        }
        assert_eq!(pids(&app), [2], "the filter recomputes");

        let model = app.model.clone();
        let before = app.list_computations.get();
        app.apply_model(model);
        assert_eq!(pids(&app), [2]);
        assert_eq!(
            app.list_computations.get(),
            before + 1,
            "a new sample recomputes"
        );
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
        assert_eq!(app.tab(), Tab::Containers);
        app.handle_key(Key::Tab);
        assert_eq!(app.tab(), Tab::Processes);
        app.handle_key(Key::Char('2'));
        assert_eq!(app.tab(), Tab::Services);
        app.handle_key(Key::Char('9'));
        assert_eq!(app.tab(), Tab::Containers);
        app.handle_key(Key::Char('0'));
        assert_eq!(app.tab(), Tab::Containers, "there is no tab 0");
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
        app.set_logs("z.service", Ok("line 1\nline 2".to_owned()));
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
