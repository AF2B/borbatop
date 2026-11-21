//! The interactive loop: terminal setup, events, commands, and teardown.

use std::io;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use borbatop_core::{Nice, Pid, ProcessId};
use borbatop_sources::{ScheduleError, send_signal, set_affinity, set_nice};
use borbatop_tui::{
    App, Command, MessageKind, Model, Seek, SignalChoice, UiConfig, draw, is_interrupt,
    key_from_event, mouse_from_event,
};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use ratatui::crossterm::execute;

use crate::config::Settings;
use crate::convert::{signal_of, unit_action_of};
use crate::reports::ProcessUsage;
use crate::worker::{
    WorkerEvent, spawn_detail, spawn_logs, spawn_pacman, spawn_report, spawn_sampler,
    spawn_unit_action,
};

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
    session(settings, new_app(settings), &events, &received, None)
}

/// Opens the screen over a recording and runs until the person quits.
///
/// The samples advance one per interval; Space pauses and the left and right arrows step.
///
/// # Arguments
///
/// * `settings` - validated settings; the interval paces the playback
/// * `models` - the recorded samples, oldest first; must not be empty
///
/// # Errors
///
/// Returns the I/O error that stopped the terminal.
pub fn replay(settings: Settings, models: Vec<Model>) -> io::Result<()> {
    let (events, received) = channel();
    let mut app = new_app(settings);
    let replay = Replay::new(models, settings.interval);
    replay.show(&mut app);
    session(settings, app, &events, &received, Some(replay))
}

fn new_app(settings: Settings) -> App {
    App::new(UiConfig {
        theme: settings.theme,
        tree: settings.tree,
        show_package: settings.show_package,
        history_len: settings.history_len,
        alerts: settings.alerts,
    })
}

fn session(
    settings: Settings,
    app: App,
    events: &Sender<WorkerEvent>,
    received: &Receiver<WorkerEvent>,
    replay: Option<Replay>,
) -> io::Result<()> {
    let mut terminal = ratatui::try_init()?;
    let result = capture_mouse(settings.mouse)
        .and_then(|()| event_loop(&mut terminal, app, events, received, replay));
    // Best effort: the terminal is being restored, and there is nobody left to tell.
    let _released = capture_mouse_release(settings.mouse);
    ratatui::restore();
    result
}

/// A recording being played back: where it is and when the next sample is due.
struct Replay {
    models: Vec<Model>,
    position: usize,
    interval: Duration,
    shown_at: Instant,
}

impl Replay {
    fn new(models: Vec<Model>, interval: Duration) -> Self {
        Self {
            models,
            position: 0,
            interval,
            shown_at: Instant::now(),
        }
    }

    /// Puts the sample at the current position on the screen.
    fn show(&self, app: &mut App) {
        if let Some(model) = self.models.get(self.position) {
            app.show_model(model.clone());
            app.set_replay(self.position, self.models.len());
        }
    }

    fn seek(&mut self, direction: Seek, app: &mut App) {
        let last = self.models.len().saturating_sub(1);
        self.position = match direction {
            Seek::Backward => self.position.saturating_sub(1),
            Seek::Forward => self.position.saturating_add(1).min(last),
        };
        self.shown_at = Instant::now();
        self.show(app);
    }

    /// Moves to the next sample when it is due; a paused screen and the last sample stay put.
    fn tick(&mut self, app: &mut App) {
        let has_next = self.position.saturating_add(1) < self.models.len();
        if !app.is_paused() && has_next && self.shown_at.elapsed() >= self.interval {
            self.seek(Seek::Forward, app);
        }
    }
}

/// Asks the terminal to report clicks and the wheel, when wanted.
fn capture_mouse(wanted: bool) -> io::Result<()> {
    if wanted {
        execute!(io::stdout(), EnableMouseCapture)?;
    }
    Ok(())
}

fn capture_mouse_release(wanted: bool) -> io::Result<()> {
    if wanted {
        execute!(io::stdout(), DisableMouseCapture)?;
    }
    Ok(())
}

fn event_loop(
    terminal: &mut DefaultTerminal,
    mut app: App,
    events: &Sender<WorkerEvent>,
    received: &Receiver<WorkerEvent>,
    mut replay: Option<Replay>,
) -> io::Result<()> {
    loop {
        terminal.draw(|frame| draw(frame, &app))?;

        if event::poll(INPUT_POLL)? {
            let command = match event::read()? {
                Event::Key(key) => {
                    if is_interrupt(&key) {
                        return Ok(());
                    }
                    key_from_event(key).and_then(|key| app.handle_key(key))
                }
                Event::Mouse(mouse) => {
                    mouse_from_event(mouse).and_then(|mouse| app.handle_mouse(mouse))
                }
                Event::FocusGained | Event::FocusLost | Event::Paste(_) | Event::Resize(_, _) => {
                    None
                }
            };
            if let Some(command) = command
                && !dispatch(command, &mut app, events, replay.as_mut())
            {
                return Ok(());
            }
        }

        if let Some(replay) = replay.as_mut() {
            replay.tick(&mut app);
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
        WorkerEvent::Logs { unit, result } => app.set_logs(&unit, result),
        WorkerEvent::Overlay { title, result } => app.set_overlay(title, result),
        WorkerEvent::Report { tab, result } => app.set_report(tab, result),
        WorkerEvent::Outcome(Ok(text)) => app.notify(text, MessageKind::Info),
        WorkerEvent::Outcome(Err(text)) => app.notify(text, MessageKind::Error),
    }
}

/// Routes a command: steps a recording, refuses to act on recorded data, or runs it for real.
/// `false` means the program must quit.
fn dispatch(
    command: Command,
    app: &mut App,
    events: &Sender<WorkerEvent>,
    replay: Option<&mut Replay>,
) -> bool {
    let Some(replay) = replay else {
        return execute(command, app, events);
    };
    match command {
        Command::Quit => return false,
        Command::Seek(direction) => replay.seek(direction, app),
        Command::LoadReport(_) | Command::RefreshPacman | Command::LoadLogs { .. } => {
            return execute(command, app, events);
        }
        Command::SendSignal { .. }
        | Command::SetNice { .. }
        | Command::SetAffinity { .. }
        | Command::ControlUnit { .. }
        | Command::LoadDetail { .. } => app.notify(
            "this is a recording: nothing here can be acted on",
            MessageKind::Error,
        ),
    }
    true
}

/// Runs a command; `false` means the program must quit.
fn execute(command: Command, app: &mut App, events: &Sender<WorkerEvent>) -> bool {
    match command {
        Command::Quit => return false,
        // Only a recording has samples to step through.
        Command::Seek(_) => {}
        Command::SendSignal { id, signal } => deliver(app, id, signal),
        Command::SetNice { id, nice } => renice(app, id, nice),
        Command::SetAffinity { id, cpus } => pin(app, id, &cpus),
        Command::ControlUnit { unit, action } => {
            let started = spawn_unit_action(events.clone(), unit, unit_action_of(action));
            report_start(app, "start systemctl", started);
        }
        Command::LoadDetail { id, label } => {
            report_start(
                app,
                "read process details",
                spawn_detail(events.clone(), id, label),
            );
        }
        Command::LoadReport(tab) => {
            let started = spawn_report(events.clone(), tab, usage_of(app.model()));
            report_start(app, &format!("build the {} report", tab.label()), started);
        }
        Command::RefreshPacman => {
            let started = spawn_pacman(events.clone(), pacman_inputs(app.model()));
            report_start(app, "start the pacman report", started);
        }
        Command::LoadLogs { unit } => {
            report_start(app, "start journalctl", spawn_logs(events.clone(), unit));
        }
    }
    true
}

/// Tells the person when a background thread could not be started.
fn report_start(app: &mut App, what: &str, started: io::Result<()>) {
    if let Err(error) = started {
        app.notify(format!("cannot {what}: {error}"), MessageKind::Error);
    }
}

/// CPU and memory of every process, to total up per container.
fn usage_of(model: &Model) -> Vec<ProcessUsage> {
    model
        .processes
        .iter()
        .map(|process| ProcessUsage {
            unit: process.unit.clone(),
            cpu: process.cpu,
            memory: process.memory,
        })
        .collect()
}

/// `(pid, program name)` of every process, to look for stale libraries.
fn pacman_inputs(model: &Model) -> Vec<(Pid, String)> {
    model
        .processes
        .iter()
        .map(|process| (process.pid, process_name(&process.command)))
        .collect()
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
