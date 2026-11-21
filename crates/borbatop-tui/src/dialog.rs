//! Modal dialogs: what they hold, how keys change them, and what they ask for on confirmation.
//!
//! A dialog knows nothing about colors: it describes its lines with a [`Tone`] and the drawing
//! code maps tones to styles.

use borbatop_core::{Nice, ProcessId};

use crate::input::{Command, Key, ServiceAction, SignalChoice};
use crate::widgets::fit;

const NICE_FINE_STEP: i8 = 1;
const NICE_COARSE_STEP: i8 = 5;
const CPUS_PER_ROW: usize = 6;
const TARGET_RESERVED: usize = 12;
const KEY_ALL_CPUS: char = 'a';
const KEY_TOGGLE_CPU: char = ' ';

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

/// The open modal dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Dialog {
    Signal(SignalDialog),
    Nice(NiceDialog),
    Affinity(AffinityDialog),
    Service(ServiceDialog),
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
        }
    }

    /// The text of the dialog, for a body `width` cells wide.
    pub(crate) fn view(&self, width: usize) -> DialogView {
        match self {
            Self::Signal(dialog) => dialog.view(width),
            Self::Nice(dialog) => dialog.view(width),
            Self::Affinity(dialog) => dialog.view(width),
            Self::Service(dialog) => dialog.view(width),
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
            Key::Char(_) | Key::Escape | Key::Backspace | Key::Tab | Key::BackTab => 0,
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
