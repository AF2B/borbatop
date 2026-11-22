//! The interactive loop: terminal setup, events, commands, and teardown.

use std::io::{self, BufRead};
use std::process::{Command as Program, ExitStatus};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant, SystemTime};

use borbatop_core::{Nice, Pid, ProcessId};
use borbatop_sources::{
    ScheduleError, UpdateCommand, send_signal, set_affinity, set_nice, update_command,
};
use borbatop_tui::{
    App, Command, ExportRequest, Key, MessageKind, Model, Seek, SignalChoice, UiConfig, UpdatePlan,
    draw, is_interrupt, key_from_event, mouse_from_event,
};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use ratatui::crossterm::execute;

use crate::config::Settings;
use crate::convert::{container_action_of, signal_of, unit_action_of, update_request_of};
use crate::export::write_export;
use crate::reports::ProcessUsage;
use crate::worker::{
    WorkerEvent, spawn_container_action, spawn_container_logs, spawn_containers, spawn_detail,
    spawn_logs, spawn_pacman, spawn_report, spawn_sampler, spawn_unit_action,
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
        .and_then(|()| event_loop(&mut terminal, app, events, received, replay, settings.mouse));
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
    mouse: bool,
) -> io::Result<()> {
    loop {
        terminal.draw(|frame| draw(frame, &app))?;

        if event::poll(INPUT_POLL)? {
            let event = event::read()?;
            if let Some(command) = command_from(&event, &mut app) {
                if let Command::RunUpdate(plan) = &command {
                    if replay.is_some() {
                        app.notify(
                            "this is a recording: nothing here can be acted on",
                            MessageKind::Error,
                        );
                    } else {
                        update_on_terminal(terminal, &mut app, (events, received), plan, mouse)?;
                    }
                } else if !dispatch(command, &mut app, events, replay.as_mut()) {
                    return Ok(());
                }
            }
        }
        continue_after_input(replay.as_mut(), &mut app, received);
    }
}

/// What a terminal event asks the program to do, if anything. Ctrl+C quits at once.
fn command_from(event: &Event, app: &mut App) -> Option<Command> {
    match event {
        Event::Key(key) if is_interrupt(key) => Some(Command::Quit),
        Event::Key(key) => key_from_event(*key).and_then(|key| app.handle_key(key)),
        Event::Mouse(mouse) => mouse_from_event(*mouse).and_then(|mouse| app.handle_mouse(mouse)),
        Event::FocusGained | Event::FocusLost | Event::Paste(_) | Event::Resize(_, _) => None,
    }
}

/// Between inputs: advance a replay when due and show what the workers sent meanwhile.
fn continue_after_input(
    replay: Option<&mut Replay>,
    app: &mut App,
    received: &Receiver<WorkerEvent>,
) {
    if let Some(replay) = replay {
        replay.tick(app);
    }
    while let Ok(update) = received.try_recv() {
        apply(update, app);
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
        WorkerEvent::Containers(result) => app.set_containers(result.map(|view| *view)),
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
        Command::LoadReport(_)
        | Command::RefreshPacman
        | Command::LoadLogs { .. }
        | Command::LoadContainers
        | Command::LoadContainerLogs { .. }
        | Command::Export(_) => {
            return execute(command, app, events);
        }
        Command::SendSignal { .. }
        | Command::SetNice { .. }
        | Command::SetAffinity { .. }
        | Command::ControlUnit { .. }
        | Command::ControlContainer { .. }
        | Command::RunUpdate(_)
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
        // Only a recording has samples to step through, and the event loop, which owns the
        // terminal, handles updates before a command reaches this point.
        Command::Seek(_) | Command::RunUpdate(_) => {}
        Command::SendSignal { id, signal } => deliver(app, id, signal),
        Command::SetNice { id, nice } => renice(app, id, nice),
        Command::SetAffinity { id, cpus } => pin(app, id, &cpus),
        Command::Export(request) => export(app, &request),
        Command::ControlUnit { .. }
        | Command::LoadLogs { .. }
        | Command::LoadDetail { .. }
        | Command::LoadReport(_)
        | Command::RefreshPacman
        | Command::LoadContainers
        | Command::ControlContainer { .. }
        | Command::LoadContainerLogs { .. } => spawn_background(command, app, events),
    }
    true
}

/// Starts the worker thread that carries out a slow command and reports back by event.
fn spawn_background(command: Command, app: &mut App, events: &Sender<WorkerEvent>) {
    let (what, started) = match command {
        Command::ControlUnit { unit, action } => (
            "start systemctl".to_owned(),
            spawn_unit_action(events.clone(), unit, unit_action_of(action)),
        ),
        Command::LoadLogs { unit } => (
            "start journalctl".to_owned(),
            spawn_logs(events.clone(), unit),
        ),
        Command::LoadDetail { id, label } => (
            "read process details".to_owned(),
            spawn_detail(events.clone(), id, label),
        ),
        Command::LoadReport(tab) => (
            format!("build the {} report", tab.label()),
            spawn_report(events.clone(), tab),
        ),
        Command::RefreshPacman => (
            "start the pacman report".to_owned(),
            spawn_pacman(events.clone(), pacman_inputs(app.model())),
        ),
        Command::LoadContainers => (
            "list the containers".to_owned(),
            spawn_containers(events.clone(), usage_of(app.model())),
        ),
        Command::ControlContainer {
            runtime,
            id,
            action,
        } => (
            "run the container command".to_owned(),
            spawn_container_action(
                events.clone(),
                runtime,
                id,
                container_action_of(action),
                usage_of(app.model()),
            ),
        ),
        Command::LoadContainerLogs { runtime, id, name } => (
            "read the container logs".to_owned(),
            spawn_container_logs(events.clone(), runtime, id, name),
        ),
        // The other commands never reach this function.
        Command::Quit
        | Command::Seek(_)
        | Command::SendSignal { .. }
        | Command::SetNice { .. }
        | Command::SetAffinity { .. }
        | Command::Export(_)
        | Command::RunUpdate(_) => return,
    };
    report_start(app, &what, started);
}

/// Runs an update on the real terminal: the screen steps aside, the command talks to the person
/// directly (password, confirmation), and the screen comes back and refreshes the report.
fn update_on_terminal(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    (events, received): (&Sender<WorkerEvent>, &Receiver<WorkerEvent>),
    plan: &UpdatePlan,
    mouse: bool,
) -> io::Result<()> {
    let command = match update_command(&update_request_of(plan)) {
        Ok(command) => command,
        Err(error) => {
            app.notify(format!("update refused: {error}"), MessageKind::Error);
            return Ok(());
        }
    };

    capture_mouse_release(mouse)?;
    ratatui::restore();
    let status = run_on_terminal(&command);
    *terminal = ratatui::try_init()?;
    capture_mouse(mouse)?;

    // The sampler kept running meanwhile: catch up first, or the samples queued during a long
    // update would make the result message fade the moment it appears.
    while let Ok(update) = received.try_recv() {
        apply(update, app);
    }
    report_update(app, &command, status);
    if let Some(refresh) = app.handle_key(Key::Char('R')) {
        execute(refresh, app, events);
    }
    Ok(())
}

/// Runs the command with the terminal's own input and output, then waits for Enter so the person
/// can read what it printed.
fn run_on_terminal(command: &UpdateCommand) -> io::Result<ExitStatus> {
    println!(
        "
borbatop: running `{command}`
"
    );
    let status = Program::new(command.program())
        .args(command.arguments())
        .status();
    println!(
        "
borbatop: press Enter to return to the screen."
    );
    // Any answer, or none at all (input closed), brings the screen back.
    let _answer = io::stdin().lock().lines().next();
    status
}

fn report_update(app: &mut App, command: &UpdateCommand, status: io::Result<ExitStatus>) {
    match status {
        Ok(status) if status.success() => {
            app.notify(format!("`{command}` finished"), MessageKind::Info);
        }
        Ok(status) => app.notify(
            format!("`{command}` ended with {status}"),
            MessageKind::Error,
        ),
        Err(error) => app.notify(
            format!("cannot run {}: {error}", command.program()),
            MessageKind::Error,
        ),
    }
}

/// Writes the tab being shown to a JSON file in the current directory and says where it went.
fn export(app: &mut App, request: &ExportRequest) {
    let written = std::env::current_dir()
        .map_err(|error| format!("cannot find the current directory: {error}"))
        .and_then(|directory| {
            write_export(&directory, request, app.model(), SystemTime::now())
                .map_err(|error| error.to_string())
        });
    match written {
        Ok(path) => app.notify(format!("exported to {}", path.display()), MessageKind::Info),
        Err(reason) => app.notify(format!("export failed: {reason}"), MessageKind::Error),
    }
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
