//! Background threads: the sampler, the pacman report and the journal reader.
//!
//! The screen never waits for I/O: every slow read runs here and arrives as a [`WorkerEvent`].

use std::io;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread;

use borbatop_core::{Pid, ProcessId};
use borbatop_sources::{
    JournalLevel, SystemCollector, UnitAction, collect_connections, collect_gpus,
    collect_pacman_report, control_unit, list_containers, list_units, process_detail,
    startup_blame, startup_summary, system_journal, unit_logs,
};
use borbatop_tui::{Model, PacmanView, Report, Tab};

use crate::config::Settings;
use crate::convert::pacman_view;
use crate::model::ModelBuilder;
use crate::reports::{
    ProcessUsage, boot_report, connections_report, containers_report, detail_text, gpu_report,
    logs_report,
};

const SERVICES_EVERY_SAMPLES: u32 = 5;
const JOURNAL_LINES: u16 = 300;
const SYSTEM_JOURNAL_LINES: u16 = 100;
const OSRELEASE: &str = "/proc/sys/kernel/osrelease";

/// What a background thread reports to the screen.
pub enum WorkerEvent {
    /// A new sample.
    Model(Box<Model>),
    /// A sample could not be taken.
    Failure(String),
    /// A freshly built Pacman report.
    Pacman(Box<PacmanView>),
    /// The journal of a unit, or why it could not be read.
    Logs {
        /// Unit the text belongs to.
        unit: String,
        /// Journal text, or the failure message.
        result: Result<String, String>,
    },
    /// The outcome of an action: a confirmation, or why it failed.
    Outcome(Result<String, String>),
    /// Text for the overlay: everything known about one process, or why it is unknown.
    Overlay {
        /// What the text is about.
        title: String,
        /// The text, or the failure message.
        result: Result<String, String>,
    },
    /// The report of an on-demand tab.
    Report {
        /// The tab it belongs to.
        tab: Tab,
        /// The report, or why it could not be built.
        result: Result<Report, String>,
    },
}

/// Keeps the sampler running; dropping it asks the thread to stop.
pub struct SamplerHandle {
    _stop: Sender<()>,
}

/// Starts the sampler thread.
///
/// The thread loads the user and package tables first, so the screen opens at once and the
/// first sample arrives a moment later.
///
/// # Arguments
///
/// * `settings` - sampling interval and history length
/// * `events` - where samples are sent
///
/// # Errors
///
/// Returns the OS error if the thread cannot be created.
pub fn spawn_sampler(settings: Settings, events: Sender<WorkerEvent>) -> io::Result<SamplerHandle> {
    let (stop, stop_signal) = channel();
    thread::Builder::new()
        .name("borbatop-sampler".to_owned())
        .spawn(move || sample_forever(settings, &events, &stop_signal))?;
    Ok(SamplerHandle { _stop: stop })
}

fn sample_forever(settings: Settings, events: &Sender<WorkerEvent>, stop: &Receiver<()>) {
    let (mut collector, page_size) = match SystemCollector::system()
        .and_then(|collector| SystemCollector::page_size().map(|page_size| (collector, page_size)))
    {
        Ok(ready) => ready,
        Err(error) => {
            let _delivered = events.send(WorkerEvent::Failure(error.to_string()));
            return;
        }
    };
    // The kernel release only decorates the header: when unreadable, the header omits it.
    let kernel = std::fs::read_to_string(OSRELEASE)
        .map_or_else(|_| String::new(), |text| text.trim().to_owned());
    let mut builder = ModelBuilder::new(settings.history_len, page_size, kernel);
    let mut sample_number = 0_u32;

    loop {
        if sample_number.is_multiple_of(SERVICES_EVERY_SAMPLES) {
            builder.set_services(list_units().map_err(|error| error.to_string()));
        }
        let event = match collector.collect() {
            Ok(snapshot) => WorkerEvent::Model(Box::new(builder.build(&snapshot))),
            Err(error) => WorkerEvent::Failure(error.to_string()),
        };
        if events.send(event).is_err() {
            return;
        }
        sample_number = sample_number.wrapping_add(1);

        match stop.recv_timeout(settings.interval) {
            Err(RecvTimeoutError::Timeout) => {}
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Builds the Pacman report on its own thread.
///
/// # Arguments
///
/// * `events` - where the report is sent
/// * `processes` - `(pid, name)` of the running processes, to look for stale libraries
///
/// # Errors
///
/// Returns the OS error if the thread cannot be created.
pub fn spawn_pacman(events: Sender<WorkerEvent>, processes: Vec<(Pid, String)>) -> io::Result<()> {
    thread::Builder::new()
        .name("borbatop-pacman".to_owned())
        .spawn(move || {
            let view = pacman_view(&collect_pacman_report(&processes));
            // The receiver is gone only when the program is quitting.
            let _delivered = events.send(WorkerEvent::Pacman(Box::new(view)));
        })
        .map(|_handle| ())
}

/// Reads the journal of `unit` on its own thread.
///
/// # Arguments
///
/// * `events` - where the text is sent
/// * `unit` - unit to read
///
/// # Errors
///
/// Returns the OS error if the thread cannot be created.
pub fn spawn_logs(events: Sender<WorkerEvent>, unit: String) -> io::Result<()> {
    thread::Builder::new()
        .name("borbatop-journal".to_owned())
        .spawn(move || {
            let result = unit_logs(&unit, JOURNAL_LINES).map_err(|error| error.to_string());
            let _delivered = events.send(WorkerEvent::Logs { unit, result });
        })
        .map(|_handle| ())
}

/// Reads everything known about one process on its own thread.
///
/// # Arguments
///
/// * `events` - where the text is sent
/// * `id` - process to inspect: PID plus start time
/// * `label` - short name for the overlay title
///
/// # Errors
///
/// Returns the OS error if the thread cannot be created.
pub fn spawn_detail(events: Sender<WorkerEvent>, id: ProcessId, label: String) -> io::Result<()> {
    thread::Builder::new()
        .name("borbatop-detail".to_owned())
        .spawn(move || {
            let result = process_detail(id)
                .map(|detail| detail_text(&detail))
                .map_err(|error| error.to_string());
            let _delivered = events.send(WorkerEvent::Overlay {
                title: format!("PROCESS · {label}"),
                result,
            });
        })
        .map(|_handle| ())
}

/// Builds the report of an on-demand tab on its own thread.
///
/// # Arguments
///
/// * `events` - where the report is sent
/// * `tab` - the tab to build
/// * `usage` - the running processes, to total up per container
///
/// # Errors
///
/// Returns the OS error if the thread cannot be created.
pub fn spawn_report(
    events: Sender<WorkerEvent>,
    tab: Tab,
    usage: Vec<ProcessUsage>,
) -> io::Result<()> {
    thread::Builder::new()
        .name("borbatop-report".to_owned())
        .spawn(move || {
            let result = build_report(tab, &usage);
            let _delivered = events.send(WorkerEvent::Report { tab, result });
        })
        .map(|_handle| ())
}

fn borrowed(result: &Result<String, String>) -> Result<&str, &str> {
    result.as_ref().map(String::as_str).map_err(String::as_str)
}

fn build_report(tab: Tab, usage: &[ProcessUsage]) -> Result<Report, String> {
    match tab {
        Tab::Connections => collect_connections()
            .map(|report| connections_report(&report))
            .map_err(|error| error.to_string()),
        Tab::Boot => {
            let summary = startup_summary().map_err(|error| error.to_string());
            let blame = startup_blame().map_err(|error| error.to_string());
            Ok(boot_report(
                borrowed(&summary),
                blame.as_ref().map_err(String::as_str),
            ))
        }
        Tab::Logs => {
            let journal = |level| {
                system_journal(level, SYSTEM_JOURNAL_LINES).map_err(|error| error.to_string())
            };
            let errors = journal(JournalLevel::Errors);
            let warnings = journal(JournalLevel::Warnings);
            Ok(logs_report(borrowed(&errors), borrowed(&warnings)))
        }
        Tab::Gpu => Ok(gpu_report(&collect_gpus())),
        Tab::Containers => Ok(containers_report(&list_containers(), usage)),
        Tab::Processes | Tab::Services | Tab::Pacman | Tab::Io => {
            Err(format!("{} has no on-demand report", tab.label()))
        }
    }
}

/// Runs a start, stop or restart on its own thread: `systemctl` can take seconds.
///
/// # Arguments
///
/// * `events` - where the outcome is sent
/// * `unit` - unit to act on
/// * `action` - what to do with it
///
/// # Errors
///
/// Returns the OS error if the thread cannot be created.
pub fn spawn_unit_action(
    events: Sender<WorkerEvent>,
    unit: String,
    action: UnitAction,
) -> io::Result<()> {
    thread::Builder::new()
        .name("borbatop-systemctl".to_owned())
        .spawn(move || {
            let verb = action.verb();
            let outcome = control_unit(&unit, action)
                .map(|()| format!("{verb} {unit}: done"))
                .map_err(|error| format!("{verb} {unit} failed: {error}"));
            let _delivered = events.send(WorkerEvent::Outcome(outcome));
        })
        .map(|_handle| ())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::mpsc::RecvTimeoutError as Timeout;
    use std::time::Duration;

    use borbatop_tui::ThemeKind;

    use super::*;

    const PATIENCE: Duration = Duration::from_secs(20);

    fn settings(interval_ms: u64) -> Settings {
        Settings {
            interval: Duration::from_millis(interval_ms),
            theme: ThemeKind::Borba,
            tree: false,
            show_package: true,
            mouse: true,
            history_len: 8,
            alerts: borbatop_tui::AlertThresholds::default(),
        }
    }

    #[test]
    fn sampler_streams_models_until_its_handle_is_dropped() {
        let (events, received) = channel();
        let handle = spawn_sampler(settings(200), events).unwrap();

        for _ in 0..2 {
            match received.recv_timeout(PATIENCE).unwrap() {
                WorkerEvent::Model(model) => assert_ne!(model.processes.len(), 0),
                WorkerEvent::Failure(reason) => panic!("sampler failed: {reason}"),
                WorkerEvent::Pacman(_)
                | WorkerEvent::Logs { .. }
                | WorkerEvent::Outcome(_)
                | WorkerEvent::Overlay { .. }
                | WorkerEvent::Report { .. } => panic!("unexpected event"),
            }
        }
        drop(handle);
        let mut drained = 0;
        loop {
            match received.recv_timeout(Duration::from_secs(2)) {
                Ok(_) => drained += 1,
                Err(Timeout::Disconnected) => break,
                Err(Timeout::Timeout) => panic!("sampler did not stop"),
            }
            assert!(drained < 10, "sampler kept running");
        }
    }

    #[test]
    fn pacman_report_arrives_as_an_event() {
        let (events, received) = channel();
        spawn_pacman(events, Vec::new()).unwrap();
        assert!(matches!(
            received.recv_timeout(PATIENCE).unwrap(),
            WorkerEvent::Pacman(_)
        ));
    }

    #[test]
    fn every_on_demand_tab_builds_a_report_on_this_machine() {
        for tab in Tab::ALL.into_iter().filter(|tab| tab.is_report()) {
            let report = build_report(tab, &[]).unwrap();
            assert!(!report.lines.is_empty(), "{} is empty", tab.label());
        }
        assert!(build_report(Tab::Processes, &[]).is_err());
    }

    #[test]
    fn process_detail_arrives_as_an_overlay_event() {
        let (events, received) = channel();
        let me = borbatop_core::Pid::new(std::process::id());
        let sample = std::fs::read_to_string(format!("/proc/{me}/stat")).unwrap();
        let id = borbatop_core::parse_process_stat(&sample).unwrap().id();

        spawn_detail(events, id, "me".to_owned()).unwrap();

        match received.recv_timeout(PATIENCE).unwrap() {
            WorkerEvent::Overlay { title, result } => {
                assert_eq!(title, "PROCESS · me");
                let text = result.unwrap();
                assert!(text.contains("Executable"), "{text}");
                assert!(text.contains("Descriptors"), "{text}");
            }
            WorkerEvent::Model(_)
            | WorkerEvent::Failure(_)
            | WorkerEvent::Pacman(_)
            | WorkerEvent::Logs { .. }
            | WorkerEvent::Outcome(_)
            | WorkerEvent::Report { .. } => panic!("unexpected event"),
        }
    }

    #[test]
    fn an_invalid_unit_name_is_reported_as_a_failed_outcome() {
        let (events, received) = channel();
        spawn_unit_action(events, "--bad".to_owned(), UnitAction::Stop).unwrap();
        match received.recv_timeout(PATIENCE).unwrap() {
            WorkerEvent::Outcome(result) => {
                let message = result.unwrap_err();
                assert!(message.contains("stop --bad failed"), "{message}");
            }
            WorkerEvent::Model(_)
            | WorkerEvent::Failure(_)
            | WorkerEvent::Pacman(_)
            | WorkerEvent::Logs { .. }
            | WorkerEvent::Overlay { .. }
            | WorkerEvent::Report { .. } => panic!("unexpected event"),
        }
    }

    #[test]
    fn journal_of_an_invalid_unit_comes_back_as_a_failure_message() {
        let (events, received) = channel();
        spawn_logs(events, "--bad".to_owned()).unwrap();
        match received.recv_timeout(PATIENCE).unwrap() {
            WorkerEvent::Logs { unit, result } => {
                assert_eq!(unit, "--bad");
                assert!(result.unwrap_err().contains("invalid unit name"));
            }
            WorkerEvent::Model(_)
            | WorkerEvent::Failure(_)
            | WorkerEvent::Pacman(_)
            | WorkerEvent::Outcome(_)
            | WorkerEvent::Overlay { .. }
            | WorkerEvent::Report { .. } => panic!("unexpected event"),
        }
    }
}
