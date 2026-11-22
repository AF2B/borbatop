//! Modal dialogs: what they hold, how keys change them, and what they ask for on confirmation.
//!
//! A dialog knows nothing about colors: it describes its lines with a [`Tone`] and the drawing
//! code maps tones to styles.

use borbatop_core::{Nice, ProcessId};

use crate::input::{Command, ContainerChoice, Key, ServiceAction, SignalChoice, UpdatePlan};
use crate::model::PacmanView;
use crate::widgets::fit;

const NICE_FINE_STEP: i8 = 1;
const NICE_COARSE_STEP: i8 = 5;
const CPUS_PER_ROW: usize = 6;
const TARGET_RESERVED: usize = 12;
const KEY_ALL_CPUS: char = 'a';
const KEY_TOGGLE_CPU: char = ' ';
const PACKAGE_ROWS: usize = 10;
const PACKAGE_CONTEXT_ROWS: usize = 5;

/// How a line of a dialog should look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tone {
    Plain,
    Secondary,
    Selected,
    Danger,
    DangerSelected,
}

/// One line of a dialog, split into runs so a cursor can stand out inside a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DialogLine {
    pub(crate) runs: Vec<(String, Tone)>,
}

impl DialogLine {
    fn plain(text: impl Into<String>, tone: Tone) -> Self {
        Self {
            runs: vec![(text.into(), tone)],
        }
    }

    fn blank() -> Self {
        Self::plain("", Tone::Plain)
    }

    /// The line as one string, without styling.
    #[cfg(test)]
    pub(crate) fn text(&self) -> String {
        self.runs.iter().map(|(text, _)| text.as_str()).collect()
    }
}

/// What a dialog shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DialogView {
    pub(crate) title: &'static str,
    pub(crate) lines: Vec<DialogLine>,
}

/// What a key press did to a dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DialogOutcome {
    /// The dialog stays open.
    Open,
    /// The dialog was cancelled.
    Closed,
    /// The dialog was confirmed and asks the program to act.
    Confirmed(Command),
    /// The dialog hands over to another one (a choice that needs a second question).
    Switch(Box<Dialog>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SignalDialog {
    pub(crate) id: ProcessId,
    pub(crate) command: String,
    pub(crate) selected: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NiceDialog {
    id: ProcessId,
    command: String,
    current: Nice,
    chosen: Nice,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AffinityDialog {
    id: ProcessId,
    command: String,
    allowed: Vec<bool>,
    cursor: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServiceDialog {
    unit: String,
    selected: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContainerDialog {
    runtime: String,
    id: String,
    name: String,
    selected: usize,
}

/// What one line of the update dialog does when chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
enum UpdateChoice {
    /// Run this update.
    Run(UpdatePlan),
    /// Pick packages of one helper first.
    Choose {
        helper: String,
        packages: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpdateDialog {
    options: Vec<(String, UpdateChoice)>,
    selected: usize,
}

/// A message under the package list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageNotice {
    None,
    NothingChosen,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PackagesDialog {
    helper: String,
    packages: Vec<String>,
    chosen: Vec<bool>,
    cursor: usize,
    notice: PackageNotice,
}

/// The open modal dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Dialog {
    Signal(SignalDialog),
    Nice(NiceDialog),
    Affinity(AffinityDialog),
    Service(ServiceDialog),
    Container(ContainerDialog),
    Update(UpdateDialog),
    Packages(PackagesDialog),
}

impl Dialog {
    /// A signal picker for one process.
    pub(crate) fn signal(id: ProcessId, command: &str) -> Self {
        Self::Signal(SignalDialog {
            id,
            command: command.to_owned(),
            selected: 0,
        })
    }

    /// A niceness editor that starts at the process's current value.
    pub(crate) fn nice(id: ProcessId, command: &str, current: Nice) -> Self {
        Self::Nice(NiceDialog {
            id,
            command: command.to_owned(),
            current,
            chosen: current,
        })
    }

    /// A CPU picker with every one of `cpu_count` CPUs allowed.
    pub(crate) fn affinity(id: ProcessId, command: &str, cpu_count: usize) -> Self {
        Self::Affinity(AffinityDialog {
            id,
            command: command.to_owned(),
            allowed: vec![true; cpu_count],
            cursor: 0,
        })
    }

    /// An action picker for one unit.
    pub(crate) fn service(unit: &str) -> Self {
        Self::Service(ServiceDialog {
            unit: unit.to_owned(),
            selected: 0,
        })
    }

    /// An action picker for one container.
    pub(crate) fn container(runtime: &str, id: &str, name: &str) -> Self {
        Self::Container(ContainerDialog {
            runtime: runtime.to_owned(),
            id: id.to_owned(),
            name: name.to_owned(),
            selected: 0,
        })
    }

    /// The update picker for what the Pacman report found, or `None` when nothing is pending.
    ///
    /// Repositories are offered as one full upgrade only (upgrading a single package leaves Arch
    /// half-upgraded); AUR helpers offer "update all" and "choose packages".
    pub(crate) fn update(view: &PacmanView) -> Option<Self> {
        let mut options = Vec::new();
        if let Ok(upgrades) = &view.upgrades
            && !upgrades.is_empty()
        {
            if let Some(tool) = &view.snapshot_tool {
                options.push((
                    format!(
                        "official repositories · {} pending · snapshot first ({tool}), then update everything",
                        upgrades.len()
                    ),
                    UpdateChoice::Run(UpdatePlan::RepositoriesAfterSnapshot { tool: tool.clone() }),
                ));
            }
            options.push((
                format!(
                    "official repositories · {} pending · update everything (sudo pacman -Syu)",
                    upgrades.len()
                ),
                UpdateChoice::Run(UpdatePlan::Repositories),
            ));
        }
        for aur in &view.aur {
            let Ok(upgrades) = &aur.upgrades else {
                continue;
            };
            if upgrades.is_empty() {
                continue;
            }
            options.push((
                format!(
                    "AUR · {} · {} pending · update all ({} -Sua)",
                    aur.helper,
                    upgrades.len(),
                    aur.helper
                ),
                UpdateChoice::Run(UpdatePlan::AurAll {
                    helper: aur.helper.clone(),
                }),
            ));
            options.push((
                format!("AUR · {} · choose packages…", aur.helper),
                UpdateChoice::Choose {
                    helper: aur.helper.clone(),
                    packages: upgrades
                        .iter()
                        .map(|upgrade| upgrade.name().to_owned())
                        .collect(),
                },
            ));
        }
        (!options.is_empty()).then_some(Self::Update(UpdateDialog {
            options,
            selected: 0,
        }))
    }

    /// Applies a key press.
    pub(crate) fn handle_key(&mut self, key: Key) -> DialogOutcome {
        if key == Key::Escape || key == Key::Char('q') {
            return DialogOutcome::Closed;
        }
        match self {
            Self::Signal(dialog) => dialog.handle_key(key),
            Self::Nice(dialog) => dialog.handle_key(key),
            Self::Affinity(dialog) => dialog.handle_key(key),
            Self::Service(dialog) => dialog.handle_key(key),
            Self::Container(dialog) => dialog.handle_key(key),
            Self::Update(dialog) => dialog.handle_key(key),
            Self::Packages(dialog) => dialog.handle_key(key),
        }
    }

    /// The text of the dialog, for a body `width` cells wide.
    pub(crate) fn view(&self, width: usize) -> DialogView {
        match self {
            Self::Signal(dialog) => dialog.view(width),
            Self::Nice(dialog) => dialog.view(width),
            Self::Affinity(dialog) => dialog.view(width),
            Self::Service(dialog) => dialog.view(width),
            Self::Container(dialog) => dialog.view(width),
            Self::Update(dialog) => dialog.view(width),
            Self::Packages(dialog) => dialog.view(width),
        }
    }
}

fn step(selected: usize, key: Key, last: usize) -> usize {
    match key {
        Key::Up | Key::Char('k') => selected.saturating_sub(1),
        Key::Down | Key::Char('j') => selected.saturating_add(1).min(last),
        Key::Home => 0,
        Key::End => last,
        Key::Char(_)
        | Key::PageUp
        | Key::PageDown
        | Key::Left
        | Key::Right
        | Key::Enter
        | Key::Escape
        | Key::Backspace
        | Key::Tab
        | Key::BackTab => selected,
    }
}

fn target_line(command: &str, detail: &str, width: usize) -> DialogLine {
    let name = fit(command, width.saturating_sub(TARGET_RESERVED));
    DialogLine {
        runs: vec![(name, Tone::Plain), (format!(" {detail}"), Tone::Secondary)],
    }
}

impl SignalDialog {
    fn handle_key(&mut self, key: Key) -> DialogOutcome {
        let last = SignalChoice::ALL.len().saturating_sub(1);
        self.selected = step(self.selected, key, last);
        if key != Key::Enter {
            return DialogOutcome::Open;
        }
        SignalChoice::ALL
            .get(self.selected)
            .map_or(DialogOutcome::Closed, |&signal| {
                DialogOutcome::Confirmed(Command::SendSignal {
                    id: self.id,
                    signal,
                })
            })
    }

    fn view(&self, width: usize) -> DialogView {
        let mut lines = vec![
            target_line(&self.command, &format!("PID {}", self.id.pid()), width),
            DialogLine::blank(),
        ];
        for (position, choice) in SignalChoice::ALL.iter().enumerate() {
            let tone = match (position == self.selected, *choice == SignalChoice::Kill) {
                (true, true) => Tone::DangerSelected,
                (true, false) => Tone::Selected,
                (false, true) => Tone::Danger,
                (false, false) => Tone::Plain,
            };
            lines.push(DialogLine::plain(
                format!(" {:<8} {} ", choice.name(), choice.effect()),
                tone,
            ));
        }
        lines.push(DialogLine::blank());
        lines.push(DialogLine::plain(
            "↑↓ choose · Enter send · Esc cancel",
            Tone::Secondary,
        ));
        DialogView {
            title: "SEND SIGNAL",
            lines,
        }
    }
}

impl NiceDialog {
    fn handle_key(&mut self, key: Key) -> DialogOutcome {
        let delta = match key {
            Key::Up | Key::Char('k') => -NICE_FINE_STEP,
            Key::Down | Key::Char('j') => NICE_FINE_STEP,
            Key::PageUp => -NICE_COARSE_STEP,
            Key::PageDown => NICE_COARSE_STEP,
            Key::Home => i8::MIN,
            Key::End => i8::MAX,
            Key::Enter => {
                return DialogOutcome::Confirmed(Command::SetNice {
                    id: self.id,
                    nice: self.chosen,
                });
            }
            Key::Char(_)
            | Key::Left
            | Key::Right
            | Key::Escape
            | Key::Backspace
            | Key::Tab
            | Key::BackTab => 0,
        };
        self.chosen = Nice::clamped(self.chosen.get().saturating_add(delta));
        DialogOutcome::Open
    }

    fn view(&self, width: usize) -> DialogView {
        let needs_privileges = self.chosen < self.current;
        let verdict = if needs_privileges {
            "raising priority needs root"
        } else {
            "lowering priority is always allowed"
        };
        DialogView {
            title: "RENICE",
            lines: vec![
                target_line(&self.command, &format!("PID {}", self.id.pid()), width),
                DialogLine::blank(),
                DialogLine::plain(
                    format!(" current {:>3}  →  new {:>3} ", self.current, self.chosen),
                    Tone::Selected,
                ),
                DialogLine::plain(
                    format!(" {verdict}"),
                    if needs_privileges {
                        Tone::Danger
                    } else {
                        Tone::Secondary
                    },
                ),
                DialogLine::blank(),
                DialogLine::plain(
                    "↑↓ ±1 · PgUp/PgDn ±5 · Enter apply · Esc cancel",
                    Tone::Secondary,
                ),
            ],
        }
    }
}

impl AffinityDialog {
    fn handle_key(&mut self, key: Key) -> DialogOutcome {
        let last = self.allowed.len().saturating_sub(1);
        self.cursor = step(self.cursor, key, last);
        match key {
            Key::Char(KEY_TOGGLE_CPU) => {
                if let Some(slot) = self.allowed.get_mut(self.cursor) {
                    *slot = !*slot;
                }
            }
            Key::Char(KEY_ALL_CPUS) => self.allowed.fill(true),
            Key::Enter => {
                let cpus: Vec<usize> = self
                    .allowed
                    .iter()
                    .enumerate()
                    .filter_map(|(cpu, &allowed)| allowed.then_some(cpu))
                    .collect();
                return DialogOutcome::Confirmed(Command::SetAffinity { id: self.id, cpus });
            }
            Key::Char(_)
            | Key::Up
            | Key::Down
            | Key::PageUp
            | Key::PageDown
            | Key::Left
            | Key::Right
            | Key::Home
            | Key::End
            | Key::Escape
            | Key::Backspace
            | Key::Tab
            | Key::BackTab => {}
        }
        DialogOutcome::Open
    }

    fn view(&self, width: usize) -> DialogView {
        let mut lines = vec![
            target_line(&self.command, &format!("PID {}", self.id.pid()), width),
            DialogLine::blank(),
        ];
        for (row, chunk) in self.allowed.chunks(CPUS_PER_ROW).enumerate() {
            let mut runs = Vec::with_capacity(chunk.len());
            for (offset, &allowed) in chunk.iter().enumerate() {
                let cpu = row.saturating_mul(CPUS_PER_ROW).saturating_add(offset);
                let mark = if allowed { 'x' } else { ' ' };
                let tone = if cpu == self.cursor {
                    Tone::Selected
                } else {
                    Tone::Plain
                };
                runs.push((format!("[{mark}]{cpu:<2} "), tone));
            }
            lines.push(DialogLine { runs });
        }
        lines.push(DialogLine::blank());
        lines.push(DialogLine::plain(
            "↑↓ move · Space toggle · a all · Enter apply · Esc cancel",
            Tone::Secondary,
        ));
        DialogView {
            title: "CPU AFFINITY",
            lines,
        }
    }
}

impl ServiceDialog {
    fn handle_key(&mut self, key: Key) -> DialogOutcome {
        let last = ServiceAction::ALL.len().saturating_sub(1);
        self.selected = step(self.selected, key, last);
        if key != Key::Enter {
            return DialogOutcome::Open;
        }
        ServiceAction::ALL
            .get(self.selected)
            .map_or(DialogOutcome::Closed, |&action| {
                DialogOutcome::Confirmed(Command::ControlUnit {
                    unit: self.unit.clone(),
                    action,
                })
            })
    }

    fn view(&self, width: usize) -> DialogView {
        let mut lines = vec![target_line(&self.unit, "", width), DialogLine::blank()];
        for (position, action) in ServiceAction::ALL.iter().enumerate() {
            let tone = match (position == self.selected, *action == ServiceAction::Stop) {
                (true, true) => Tone::DangerSelected,
                (true, false) => Tone::Selected,
                (false, true) => Tone::Danger,
                (false, false) => Tone::Plain,
            };
            lines.push(DialogLine::plain(
                format!(" {:<8} {} ", action.name(), action.effect()),
                tone,
            ));
        }
        lines.push(DialogLine::blank());
        lines.push(DialogLine::plain(
            "↑↓ choose · Enter run · Esc cancel · needs privileges",
            Tone::Secondary,
        ));
        DialogView {
            title: "SERVICE",
            lines,
        }
    }
}

impl ContainerDialog {
    fn handle_key(&mut self, key: Key) -> DialogOutcome {
        let last = ContainerChoice::ALL.len().saturating_sub(1);
        self.selected = step(self.selected, key, last);
        if key != Key::Enter {
            return DialogOutcome::Open;
        }
        ContainerChoice::ALL
            .get(self.selected)
            .map_or(DialogOutcome::Closed, |&action| {
                DialogOutcome::Confirmed(Command::ControlContainer {
                    runtime: self.runtime.clone(),
                    id: self.id.clone(),
                    action,
                })
            })
    }

    fn view(&self, width: usize) -> DialogView {
        let mut lines = vec![
            target_line(&self.name, &format!("{} {}", self.runtime, self.id), width),
            DialogLine::blank(),
        ];
        for (position, action) in ContainerChoice::ALL.iter().enumerate() {
            let destructive = matches!(action, ContainerChoice::Stop | ContainerChoice::Remove);
            let tone = match (position == self.selected, destructive) {
                (true, true) => Tone::DangerSelected,
                (true, false) => Tone::Selected,
                (false, true) => Tone::Danger,
                (false, false) => Tone::Plain,
            };
            lines.push(DialogLine::plain(
                format!(" {:<8} {} ", action.name(), action.effect()),
                tone,
            ));
        }
        lines.push(DialogLine::blank());
        if ContainerChoice::ALL.get(self.selected) == Some(&ContainerChoice::Remove) {
            lines.push(DialogLine::plain(
                "remove deletes the container: data outside volumes is lost",
                Tone::Danger,
            ));
        }
        lines.push(DialogLine::plain(
            "↑↓ choose · Enter run · Esc cancel · may need privileges",
            Tone::Secondary,
        ));
        DialogView {
            title: "CONTAINER",
            lines,
        }
    }
}

impl UpdateDialog {
    fn handle_key(&mut self, key: Key) -> DialogOutcome {
        let last = self.options.len().saturating_sub(1);
        self.selected = step(self.selected, key, last);
        if key != Key::Enter {
            return DialogOutcome::Open;
        }
        match self.options.get(self.selected) {
            Some((_, UpdateChoice::Run(plan))) => {
                DialogOutcome::Confirmed(Command::RunUpdate(plan.clone()))
            }
            Some((_, UpdateChoice::Choose { helper, packages })) => {
                DialogOutcome::Switch(Box::new(Dialog::Packages(PackagesDialog {
                    helper: helper.clone(),
                    packages: packages.clone(),
                    chosen: vec![false; packages.len()],
                    cursor: 0,
                    notice: PackageNotice::None,
                })))
            }
            None => DialogOutcome::Closed,
        }
    }

    fn view(&self, width: usize) -> DialogView {
        let mut lines = vec![
            DialogLine::plain(
                "The update runs on the real terminal: you will be asked for your password.",
                Tone::Secondary,
            ),
            DialogLine::blank(),
        ];
        for (position, (label, _)) in self.options.iter().enumerate() {
            let tone = if position == self.selected {
                Tone::Selected
            } else {
                Tone::Plain
            };
            lines.push(DialogLine::plain(
                format!(" {} ", fit(label, width.saturating_sub(2))),
                tone,
            ));
        }
        lines.push(DialogLine::blank());
        lines.push(DialogLine::plain(
            "↑↓ choose · Enter continue · Esc cancel",
            Tone::Secondary,
        ));
        DialogView {
            title: "UPDATE",
            lines,
        }
    }
}

impl PackagesDialog {
    fn handle_key(&mut self, key: Key) -> DialogOutcome {
        let last = self.packages.len().saturating_sub(1);
        self.cursor = step(self.cursor, key, last);
        match key {
            Key::Char(KEY_TOGGLE_CPU) => {
                if let Some(slot) = self.chosen.get_mut(self.cursor) {
                    *slot = !*slot;
                }
                self.notice = PackageNotice::None;
            }
            Key::Char(KEY_ALL_CPUS) => {
                self.chosen.fill(true);
                self.notice = PackageNotice::None;
            }
            Key::Enter => {
                let packages: Vec<String> = self
                    .packages
                    .iter()
                    .zip(&self.chosen)
                    .filter(|(_, chosen)| **chosen)
                    .map(|(name, _)| name.clone())
                    .collect();
                if packages.is_empty() {
                    self.notice = PackageNotice::NothingChosen;
                    return DialogOutcome::Open;
                }
                return DialogOutcome::Confirmed(Command::RunUpdate(UpdatePlan::AurPackages {
                    helper: self.helper.clone(),
                    packages,
                }));
            }
            Key::Char(_)
            | Key::Up
            | Key::Down
            | Key::PageUp
            | Key::PageDown
            | Key::Left
            | Key::Right
            | Key::Home
            | Key::End
            | Key::Escape
            | Key::Backspace
            | Key::Tab
            | Key::BackTab => {}
        }
        DialogOutcome::Open
    }

    fn view(&self, width: usize) -> DialogView {
        let first = self
            .cursor
            .saturating_sub(PACKAGE_CONTEXT_ROWS)
            .min(self.packages.len().saturating_sub(PACKAGE_ROWS));
        let mut lines = vec![
            DialogLine::plain(
                format!(
                    "AUR · {} · {} chosen",
                    self.helper,
                    self.chosen.iter().filter(|c| **c).count()
                ),
                Tone::Secondary,
            ),
            DialogLine::blank(),
        ];
        for (position, name) in self
            .packages
            .iter()
            .enumerate()
            .skip(first)
            .take(PACKAGE_ROWS)
        {
            let mark = if self.chosen.get(position) == Some(&true) {
                'x'
            } else {
                ' '
            };
            let tone = if position == self.cursor {
                Tone::Selected
            } else {
                Tone::Plain
            };
            lines.push(DialogLine::plain(
                format!(" [{mark}] {} ", fit(name, width.saturating_sub(8))),
                tone,
            ));
        }
        lines.push(DialogLine::blank());
        if self.notice == PackageNotice::NothingChosen {
            lines.push(DialogLine::plain(
                "choose at least one package",
                Tone::Danger,
            ));
        }
        lines.push(DialogLine::plain(
            "↑↓ move · Space toggle · a all · Enter update · Esc cancel",
            Tone::Secondary,
        ));
        DialogView {
            title: "AUR PACKAGES",
            lines,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use borbatop_core::{Pid, StartTime};

    use super::*;

    fn id() -> ProcessId {
        ProcessId::new(Pid::new(42), StartTime::new(7))
    }

    #[test]
    fn escape_and_q_close_every_dialog() {
        let dialogs = [
            Dialog::signal(id(), "tool"),
            Dialog::nice(id(), "tool", Nice::clamped(0)),
            Dialog::affinity(id(), "tool", 4),
            Dialog::service("sshd.service"),
            Dialog::container("docker", "abc", "web"),
        ];
        for mut dialog in dialogs {
            assert_eq!(
                dialog.clone().handle_key(Key::Escape),
                DialogOutcome::Closed
            );
            assert_eq!(dialog.handle_key(Key::Char('q')), DialogOutcome::Closed);
        }
    }

    #[test]
    fn signal_dialog_confirms_the_selected_signal() {
        let mut dialog = Dialog::signal(id(), "tool");
        assert_eq!(dialog.handle_key(Key::Down), DialogOutcome::Open);
        assert_eq!(
            dialog.handle_key(Key::Enter),
            DialogOutcome::Confirmed(Command::SendSignal {
                id: id(),
                signal: SignalChoice::Hangup,
            })
        );
    }

    #[test]
    fn nice_starts_at_the_current_value_and_clamps() {
        let mut dialog = Dialog::nice(id(), "tool", Nice::clamped(0));
        for _ in 0..3 {
            dialog.handle_key(Key::Down);
        }
        dialog.handle_key(Key::PageDown);
        assert_eq!(
            dialog.handle_key(Key::Enter),
            DialogOutcome::Confirmed(Command::SetNice {
                id: id(),
                nice: Nice::clamped(8),
            })
        );

        dialog.handle_key(Key::End);
        dialog.handle_key(Key::Down);
        assert_eq!(
            dialog.handle_key(Key::Enter),
            DialogOutcome::Confirmed(Command::SetNice {
                id: id(),
                nice: Nice::clamped(Nice::MAX),
            })
        );
        dialog.handle_key(Key::Home);
        assert_eq!(
            dialog.handle_key(Key::Enter),
            DialogOutcome::Confirmed(Command::SetNice {
                id: id(),
                nice: Nice::clamped(Nice::MIN),
            })
        );
    }

    #[test]
    fn nice_warns_when_raising_priority() {
        let mut dialog = Dialog::nice(id(), "tool", Nice::clamped(5));
        let calm = dialog.view(46);
        assert!(
            calm.lines
                .iter()
                .any(|line| line.text().contains("always allowed"))
        );

        dialog.handle_key(Key::Up);
        let warned = dialog.view(46);
        assert!(
            warned
                .lines
                .iter()
                .any(|line| line.text().contains("needs root"))
        );
    }

    #[test]
    fn affinity_starts_with_every_cpu_and_toggles_one() {
        let mut dialog = Dialog::affinity(id(), "tool", 4);
        dialog.handle_key(Key::Char(' ')); // cursor on CPU 0: remove it
        dialog.handle_key(Key::Down);
        dialog.handle_key(Key::Char(' ')); // CPU 1: remove it
        assert_eq!(
            dialog.handle_key(Key::Enter),
            DialogOutcome::Confirmed(Command::SetAffinity {
                id: id(),
                cpus: vec![2, 3],
            })
        );

        dialog.handle_key(Key::Char('a'));
        assert_eq!(
            dialog.handle_key(Key::Enter),
            DialogOutcome::Confirmed(Command::SetAffinity {
                id: id(),
                cpus: vec![0, 1, 2, 3],
            })
        );
    }

    #[test]
    fn affinity_wraps_cpus_into_rows() {
        let dialog = Dialog::affinity(id(), "tool", 8);
        let view = dialog.view(46);
        let rows: Vec<String> = view.lines.iter().map(DialogLine::text).collect();
        assert!(rows.iter().any(|row| row.starts_with("[x]0")));
        assert!(rows.iter().any(|row| row.starts_with("[x]6")));
    }

    #[test]
    fn service_dialog_offers_restart_first() {
        let mut dialog = Dialog::service("sshd.service");
        assert_eq!(
            dialog.handle_key(Key::Enter),
            DialogOutcome::Confirmed(Command::ControlUnit {
                unit: "sshd.service".to_owned(),
                action: ServiceAction::Restart,
            })
        );
        dialog.handle_key(Key::Down);
        assert_eq!(
            dialog.handle_key(Key::Enter),
            DialogOutcome::Confirmed(Command::ControlUnit {
                unit: "sshd.service".to_owned(),
                action: ServiceAction::Stop,
            })
        );
    }

    #[test]
    fn container_dialog_offers_restart_first_and_warns_before_remove() {
        let mut dialog = Dialog::container("podman", "0a1b2c3d4e5f", "web");
        let first = dialog.view(60);
        assert!(
            !first
                .lines
                .iter()
                .any(|line| line.text().contains("data outside volumes"))
        );

        for _ in 0..3 {
            dialog.handle_key(Key::Down);
        }
        let warned = dialog.view(60);
        assert!(
            warned
                .lines
                .iter()
                .any(|line| line.text().contains("data outside volumes"))
        );
        assert_eq!(
            dialog.handle_key(Key::Enter),
            DialogOutcome::Confirmed(Command::ControlContainer {
                runtime: "podman".to_owned(),
                id: "0a1b2c3d4e5f".to_owned(),
                action: ContainerChoice::Remove,
            })
        );
    }

    #[test]
    fn hostile_names_cannot_inject_escapes() {
        let dialog = Dialog::signal(id(), "evil\u{1b}[2J");
        let view = dialog.view(46);
        assert!(
            view.lines
                .iter()
                .all(|line| !line.text().chars().any(char::is_control))
        );
    }
}
