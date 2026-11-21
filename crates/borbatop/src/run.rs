//! The interactive loop: terminal setup, events, commands, and teardown.

use std::io;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use borbatop_core::{Nice, ProcessId};
use borbatop_sources::{ScheduleError, send_signal, set_affinity, set_nice};
use borbatop_tui::{
    App, Command, MessageKind, SignalChoice, UiConfig, draw, is_interrupt, key_from_event,
};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event};

use crate::config::Settings;
use crate::convert::{signal_of, unit_action_of};
use crate::worker::{WorkerEvent, spawn_logs, spawn_pacman, spawn_sampler, spawn_unit_action};

const INPUT_POLL: Duration = Duration::from_millis(100);

/// Opens the screen, runs until the person quits, and always restores the terminal.
///
/// # Arguments
///
/// * `settings` - validated settings
///
/// # Errors
///
/// Returns the I/O error that stopped the terminal.
pub fn run(settings: Settings) -> io::Result<()> {
    let (events, received) = channel();
    let _sampler = spawn_sampler(settings, events.clone())?;
    let app = App::new(UiConfig {
        theme: settings.theme,
        tree: settings.tree,
        show_package: settings.show_package,
        history_len: settings.history_len,
    });

    let mut terminal = ratatui::try_init()?;
    let result = event_loop(&mut terminal, app, &events, &received);
    ratatui::restore();
    result
}

fn event_loop(
    terminal: &mut DefaultTerminal,
    mut app: App,
    events: &Sender<WorkerEvent>,
    received: &Receiver<WorkerEvent>,
) -> io::Result<()> {
    loop {
        terminal.draw(|frame| draw(frame, &app))?;

        if event::poll(INPUT_POLL)? {
            match event::read()? {
                Event::Key(key) => {
                    if is_interrupt(&key) {
                        return Ok(());
                    }
                    let command = key_from_event(key).and_then(|key| app.handle_key(key));
                    if let Some(command) = command
                        && !execute(command, &mut app, events)
                    {
                        return Ok(());
                    }
                }
                Event::FocusGained
                | Event::FocusLost
                | Event::Mouse(_)
                | Event::Paste(_)
                | Event::Resize(_, _) => {}
            }
        }

        while let Ok(update) = received.try_recv() {
            apply(update, &mut app);
        }
    }
}

fn apply(update: WorkerEvent, app: &mut App) {
    match update {
        WorkerEvent::Model(model) => {
            app.apply_model(*model);
            app.tick();
        }
        WorkerEvent::Failure(reason) => app.notify(reason, MessageKind::Error),
        WorkerEvent::Pacman(view) => app.set_pacman(*view),
        WorkerEvent::Logs { unit, result } => app.set_logs(unit, result),
        WorkerEvent::Outcome(Ok(text)) => app.notify(text, MessageKind::Info),
        WorkerEvent::Outcome(Err(text)) => app.notify(text, MessageKind::Error),
    }
}

/// Runs a command; `false` means the program must quit.
fn execute(command: Command, app: &mut App, events: &Sender<WorkerEvent>) -> bool {
    match command {
        Command::Quit => return false,
        Command::SendSignal { id, signal } => deliver(app, id, signal),
        Command::SetNice { id, nice } => renice(app, id, nice),
        Command::SetAffinity { id, cpus } => pin(app, id, &cpus),
        Command::ControlUnit { unit, action } => {
            if let Err(error) = spawn_unit_action(events.clone(), unit, unit_action_of(action)) {
                app.notify(
                    format!("cannot start systemctl: {error}"),
                    MessageKind::Error,
                );
            }
        }
        Command::RefreshPacman => {
            let processes = app
                .model()
                .processes
                .iter()
                .map(|process| (process.pid, process_name(&process.command)))
                .collect();
            if let Err(error) = spawn_pacman(events.clone(), processes) {
                app.notify(
                    format!("cannot start pacman report: {error}"),
                    MessageKind::Error,
                );
            }
        }
        Command::LoadLogs { unit } => {
            if let Err(error) = spawn_logs(events.clone(), unit) {
                app.notify(
                    format!("cannot start journalctl: {error}"),
                    MessageKind::Error,
                );
            }
        }
    }
    true
}

fn deliver(app: &mut App, id: ProcessId, choice: SignalChoice) {
    let pid = id.pid();
    match send_signal(id, signal_of(choice)) {
        Ok(()) => app.notify(
            format!("{} sent to PID {pid}", choice.name()),
            MessageKind::Info,
        ),
        Err(error) => app.notify(
            format!("{} to PID {pid} failed: {error}", choice.name()),
            MessageKind::Error,
        ),
    }
}

fn renice(app: &mut App, id: ProcessId, nice: Nice) {
    let pid = id.pid();
    report_schedule(
        app,
        set_nice(id, nice),
        &format!("PID {pid} nice set to {nice}"),
        &format!("renice PID {pid} failed"),
    );
}

fn pin(app: &mut App, id: ProcessId, cpus: &[usize]) {
    let pid = id.pid();
    report_schedule(
        app,
        set_affinity(id, cpus),
        &format!("PID {pid} pinned to {} CPU(s)", cpus.len()),
        &format!("pinning PID {pid} failed"),
    );
}

fn report_schedule(app: &mut App, result: Result<(), ScheduleError>, done: &str, failed: &str) {
    match result {
        Ok(()) => app.notify(done, MessageKind::Info),
        Err(error) => app.notify(format!("{failed}: {error}"), MessageKind::Error),
    }
}

/// The program name in a command line: its first word, without directories.
pub(crate) fn process_name(command: &str) -> String {
    let program = command.split_whitespace().next().unwrap_or(command);
    program.rsplit('/').next().unwrap_or(program).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_name_drops_arguments_and_directories() {
        assert_eq!(process_name("/usr/bin/python /usr/bin/zapzap"), "python");
        assert_eq!(process_name("kitty"), "kitty");
        assert_eq!(process_name("[kworker/u32:2]"), "u32:2]");
        assert_eq!(process_name(""), "");
    }
}
