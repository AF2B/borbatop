//! Turning what the data sources found into the lines each tab and overlay shows.
//!
//! Everything here is pure formatting: no I/O, so each report can be tested with made-up data.

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::time::Duration;

use borbatop_core::{
    Bytes, ContainerInfo, FdKind, GpuCard, GpuVendor, Permille, SocketState, StartupBlame,
    container_id_of_unit, same_container,
};
use borbatop_sources::{Connection, ConnectionReport, GpuReport, ProcessDetail, RuntimeListing};
use borbatop_tui::{ContainerView, ContainersView, Report, ReportLine, ReportTone};

const ADDRESS_WIDTH: usize = 28;
const STATE_WIDTH: usize = 12;
const PROTOCOL_WIDTH: usize = 6;
const MAX_STARTUP_ROWS: usize = 40;
const MAX_FILE_ROWS: usize = 60;
const SLOW_UNIT: Duration = Duration::from_secs(5);
const NOTICEABLE_UNIT: Duration = Duration::from_secs(1);
const MILLIS_PER_SECOND: u128 = 1_000;
const SECONDS_PER_MINUTE: u64 = 60;
const MILLIS_PER_TENTH: u128 = 100;
const LIMIT_NAMES: [&str; 3] = ["Max open files", "Max processes", "Max locked memory"];
const NO_OWNER: &str = "—";
const SHORT_ID_LEN: usize = 12;

/// The CPU and memory of one running process, to total up per container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessUsage {
    /// Systemd unit that runs the process.
    pub unit: String,
    /// CPU usage over the last window.
    pub cpu: Permille,
    /// Resident memory.
    pub memory: Bytes,
}

fn line(text: impl Into<String>, tone: ReportTone) -> ReportLine {
    ReportLine::new(text, tone)
}

/// A duration the way systemd prints it: `889ms`, `5.2s`, `1min 3.2s`.
fn format_duration(duration: Duration) -> String {
    let millis = duration.as_millis();
    if millis < MILLIS_PER_SECOND {
        return format!("{millis}ms");
    }
    let seconds = duration.as_secs();
    let tenth = (millis % MILLIS_PER_SECOND) / MILLIS_PER_TENTH;
    let minutes = seconds / SECONDS_PER_MINUTE;
    let rest = seconds % SECONDS_PER_MINUTE;
    if minutes == 0 {
        format!("{rest}.{tenth}s")
    } else {
        format!("{minutes}min {rest}.{tenth}s")
    }
}

// ---------------------------------------------------------------- connections

fn state_tone(state: SocketState) -> ReportTone {
    match state {
        SocketState::Listen => ReportTone::Good,
        SocketState::Established => ReportTone::Normal,
        SocketState::TimeWait
        | SocketState::Close
        | SocketState::CloseWait
        | SocketState::FinWait1
        | SocketState::FinWait2
        | SocketState::LastAck
        | SocketState::Closing => ReportTone::Dim,
        SocketState::SynSent | SocketState::SynReceived | SocketState::NewSynReceived => {
            ReportTone::Normal
        }
    }
}

/// The Connections tab: every socket with the process that owns it.
///
/// # Arguments
///
/// * `report` - sockets read from `/proc/net` and matched to processes
#[must_use]
pub fn connections_report(report: &ConnectionReport) -> Report {
    let connections = sorted_connections(report);
    let listening = connections
        .iter()
        .filter(|connection| connection.entry().state() == SocketState::Listen)
        .count();

    let mut lines = vec![
        line(
            format!("{} sockets · {listening} listening", connections.len()),
            ReportTone::Heading,
        ),
        line(
            format!(
                "{:<PROTOCOL_WIDTH$} {:<STATE_WIDTH$} {:<ADDRESS_WIDTH$} {:<ADDRESS_WIDTH$} OWNER",
                "PROTO", "STATE", "LOCAL", "REMOTE"
            ),
            ReportTone::Dim,
        ),
    ];
    lines.extend(connections.into_iter().map(connection_row));
    lines.extend(connection_notes(report));
    Report { lines }
}

/// Listeners first, then by local port.
fn sorted_connections(report: &ConnectionReport) -> Vec<&Connection> {
    let mut connections: Vec<&Connection> = report.connections().iter().collect();
    connections.sort_by_key(|connection| {
        let entry = connection.entry();
        (
            entry.state() != SocketState::Listen,
            entry.local().port(),
            entry.protocol().label(),
        )
    });
    connections
}

fn connection_row(connection: &Connection) -> ReportLine {
    let entry = connection.entry();
    line(
        format!(
            "{:<PROTOCOL_WIDTH$} {:<STATE_WIDTH$} {:<ADDRESS_WIDTH$} {:<ADDRESS_WIDTH$} {}",
            entry.protocol().label(),
            entry.state().name(),
            entry.local().to_string(),
            entry.remote().to_string(),
            owner_text(connection),
        ),
        state_tone(entry.state()),
    )
}

/// What the person should know about the limits of this listing.
fn connection_notes(report: &ConnectionReport) -> Vec<ReportLine> {
    let mut notes = Vec::new();
    if report.unreadable_processes() > 0 {
        notes.push(line(
            format!(
                "{} processes cannot be inspected by this user: their sockets show no owner",
                report.unreadable_processes()
            ),
            ReportTone::Dim,
        ));
    }
    if report.missing_tables() > 0 {
        notes.push(line(
            format!(
                "{} socket tables could not be read",
                report.missing_tables()
            ),
            ReportTone::Dim,
        ));
    }
    notes
}

fn owner_text(connection: &Connection) -> String {
    let Some(first) = connection.owners().first() else {
        return NO_OWNER.to_owned();
    };
    let extra = connection.owners().len().saturating_sub(1);
    if extra == 0 {
        format!("{} {}", first.pid(), first.comm())
    } else {
        format!("{} {} +{extra}", first.pid(), first.comm())
    }
}

// --------------------------------------------------------------------- detail

/// The overlay text for one process: executable, memory, descriptors, sockets and limits.
///
/// # Arguments
///
/// * `detail` - what the kernel says about the process
#[must_use]
pub fn detail_text(detail: &ProcessDetail) -> String {
    let mut text = String::new();
    push_field(&mut text, "Executable", detail.exe().as_ref());
    push_field(&mut text, "Directory", detail.cwd().as_ref());
    push_memory(&mut text, detail);
    push_descriptors(&mut text, detail);
    push_sockets(&mut text, detail);
    push_limits(&mut text, detail);
    text
}

fn push_field<E: std::fmt::Display>(text: &mut String, label: &str, value: Result<&String, E>) {
    let shown = match value {
        Ok(value) => value.clone(),
        Err(error) => format!("unavailable ({error})"),
    };
    let _written = writeln!(text, "{label:<12} {shown}");
}

fn push_memory(text: &mut String, detail: &ProcessDetail) {
    let show = |bytes: Option<Bytes>| bytes.map_or_else(|| "n/a".to_owned(), |b| b.to_string());
    let _written = writeln!(
        text,
        "{:<12} resident {} · peak {} · swapped {}",
        "Memory",
        show(detail.resident()),
        show(detail.peak_resident()),
        show(detail.swapped()),
    );
}

fn push_descriptors(text: &mut String, detail: &ProcessDetail) {
    let descriptors = match detail.descriptors() {
        Ok(descriptors) => descriptors,
        Err(error) => {
            let _written = writeln!(text, "{:<12} unavailable ({error})", "Descriptors");
            return;
        }
    };
    let (mut files, mut sockets, mut pipes, mut other) = (0_usize, 0_usize, 0_usize, 0_usize);
    for descriptor in descriptors {
        let counter = match descriptor.kind() {
            FdKind::Path(_) => &mut files,
            FdKind::Socket(_) => &mut sockets,
            FdKind::Pipe => &mut pipes,
            FdKind::Anonymous(_) | FdKind::Other(_) => &mut other,
        };
        *counter = counter.saturating_add(1);
    }
    let _written = writeln!(
        text,
        "{:<12} {} open: {files} files · {sockets} sockets · {pipes} pipes · {other} other",
        "Descriptors",
        descriptors.len(),
    );

    let _written = writeln!(text, "\nOpen files");
    let paths = descriptors
        .iter()
        .filter_map(|descriptor| match descriptor.kind() {
            FdKind::Path(path) => Some((descriptor.fd(), path)),
            FdKind::Socket(_) | FdKind::Pipe | FdKind::Anonymous(_) | FdKind::Other(_) => None,
        });
    let mut shown = 0_usize;
    for (fd, path) in paths.take(MAX_FILE_ROWS) {
        let _written = writeln!(text, "  {fd:>4}  {path}");
        shown = shown.saturating_add(1);
    }
    if shown < files {
        let _written = writeln!(text, "  … {} more", files.saturating_sub(shown));
    }
}

fn push_sockets(text: &mut String, detail: &ProcessDetail) {
    if detail.sockets().is_empty() {
        return;
    }
    let _written = writeln!(text, "\nSockets");
    for entry in detail.sockets() {
        let _written = writeln!(
            text,
            "  {:<PROTOCOL_WIDTH$} {:<STATE_WIDTH$} {} → {}",
            entry.protocol().label(),
            entry.state().name(),
            entry.local(),
            entry.remote(),
        );
    }
}

fn push_limits(text: &mut String, detail: &ProcessDetail) {
    let Ok(limits) = detail.limits() else {
        return;
    };
    let _written = writeln!(text, "\nLimits (soft / hard)");
    for name in LIMIT_NAMES {
        if let Some(limit) = limits.iter().find(|limit| limit.name == name) {
            let _written = writeln!(
                text,
                "  {:<20} {} / {} {}",
                limit.name, limit.soft, limit.hard, limit.unit
            );
        }
    }
}

// ----------------------------------------------------------------------- boot

/// The Boot tab: the startup summary and the slowest units.
///
/// # Arguments
///
/// * `summary` - output of `systemd-analyze time`, or why it failed
/// * `blame` - the per-unit startup times, or why they are unknown
#[must_use]
pub fn boot_report(summary: Result<&str, &str>, blame: Result<&StartupBlame, &str>) -> Report {
    let mut lines = vec![line("Startup", ReportTone::Heading)];
    match summary {
        Ok(text) => lines.extend(
            text.lines()
                .filter(|text| !text.trim().is_empty())
                .map(|text| line(format!("  {text}"), ReportTone::Normal)),
        ),
        Err(reason) => lines.push(line(
            format!("  systemd-analyze: {reason}"),
            ReportTone::Bad,
        )),
    }

    lines.push(line("Slowest units", ReportTone::Heading));
    match blame {
        Ok(blame) => lines.extend(slowest_unit_lines(blame)),
        Err(reason) => lines.push(line(
            format!("  systemd-analyze blame: {reason}"),
            ReportTone::Bad,
        )),
    }
    Report { lines }
}

fn startup_tone(duration: Duration) -> ReportTone {
    if duration >= SLOW_UNIT {
        ReportTone::Bad
    } else if duration >= NOTICEABLE_UNIT {
        ReportTone::Normal
    } else {
        ReportTone::Dim
    }
}

fn slowest_unit_lines(blame: &StartupBlame) -> Vec<ReportLine> {
    let mut lines: Vec<ReportLine> = blame
        .units()
        .iter()
        .take(MAX_STARTUP_ROWS)
        .map(|startup| {
            line(
                format!(
                    "  {:>10}  {}",
                    format_duration(startup.duration()),
                    startup.unit()
                ),
                startup_tone(startup.duration()),
            )
        })
        .collect();
    let hidden = blame.units().len().saturating_sub(MAX_STARTUP_ROWS);
    if hidden > 0 {
        lines.push(line(format!("  … {hidden} more units"), ReportTone::Dim));
    }
    lines
}

// ----------------------------------------------------------------------- logs

fn journal_lines(
    title: &str,
    text: Result<&str, &str>,
    tone: ReportTone,
    lines: &mut Vec<ReportLine>,
) {
    lines.push(line(title, ReportTone::Heading));
    match text {
        Ok(text) if text.trim().is_empty() => lines.push(line("  none", ReportTone::Good)),
        Ok(text) => lines.extend(text.lines().map(|entry| {
            let tone = if entry.starts_with("-- ") {
                ReportTone::Dim
            } else {
                tone
            };
            line(format!("  {entry}"), tone)
        })),
        Err(reason) => lines.push(line(format!("  journalctl: {reason}"), ReportTone::Bad)),
    }
}

/// The Logs tab: errors and warnings of the system journal since boot.
///
/// # Arguments
///
/// * `errors` - emergency to error lines, or why they could not be read
/// * `warnings` - warning lines, or why they could not be read
#[must_use]
pub fn logs_report(errors: Result<&str, &str>, warnings: Result<&str, &str>) -> Report {
    let mut lines = Vec::new();
    journal_lines("Errors since boot", errors, ReportTone::Bad, &mut lines);
    journal_lines(
        "Warnings since boot",
        warnings,
        ReportTone::Normal,
        &mut lines,
    );
    Report { lines }
}

// ------------------------------------------------------------------------ gpu

fn vendor_name(vendor: GpuVendor) -> String {
    match vendor {
        GpuVendor::Nvidia => "NVIDIA".to_owned(),
        GpuVendor::Amd => "AMD".to_owned(),
        GpuVendor::Intel => "Intel".to_owned(),
        GpuVendor::Other(id) => format!("PCI vendor {id:#06x}"),
    }
}

fn gpu_card_lines(card: &GpuCard, lines: &mut Vec<ReportLine>) {
    let driver = card.driver.as_deref().unwrap_or("no driver");
    lines.push(line(
        format!("{} · {} · {driver}", vendor_name(card.vendor), card.name),
        ReportTone::Heading,
    ));
    let missing = "n/a";
    lines.push(line(
        format!(
            "  busy         {}",
            card.utilization_percent
                .map_or_else(|| missing.to_owned(), |percent| format!("{percent}%"))
        ),
        ReportTone::Normal,
    ));
    let memory = match (card.memory_used, card.memory_total) {
        (Some(used), Some(total)) => {
            format!(
                "{used} / {total} ({}%)",
                Permille::new(used.permille_of(total))
            )
        }
        (Some(used), None) => format!("{used}"),
        (None, Some(total)) => format!("{total} installed"),
        (None, None) => missing.to_owned(),
    };
    lines.push(line(format!("  video memory {memory}"), ReportTone::Normal));
    lines.push(line(
        format!(
            "  temperature  {}",
            card.temperature.map_or_else(
                || missing.to_owned(),
                |degrees| format!("{}°C", degrees.rounded_degrees())
            )
        ),
        ReportTone::Normal,
    ));
}

/// The GPU tab.
///
/// # Arguments
///
/// * `report` - the cards found under `/sys/class/drm` and by `nvidia-smi`
#[must_use]
pub fn gpu_report(report: &GpuReport) -> Report {
    let mut lines = Vec::new();
    for card in report.cards() {
        gpu_card_lines(card, &mut lines);
    }
    if report.cards().is_empty() {
        lines.push(line(
            "no graphics card found under /sys/class/drm",
            ReportTone::Dim,
        ));
    }
    if let Some(error) = report.nvidia_error() {
        lines.push(line(format!("nvidia-smi: {error}"), ReportTone::Bad));
    }
    Report { lines }
}

// ----------------------------------------------------------------- containers

#[derive(Default)]
struct Totals {
    processes: usize,
    cpu: u64,
    memory: u64,
}

fn totals_by_container(usage: &[ProcessUsage]) -> BTreeMap<&str, Totals> {
    let mut totals: BTreeMap<&str, Totals> = BTreeMap::new();
    for process in usage {
        if let Some(id) = container_id_of_unit(&process.unit) {
            let entry = totals.entry(id).or_default();
            entry.processes = entry.processes.saturating_add(1);
            entry.cpu = entry.cpu.saturating_add(process.cpu.get());
            entry.memory = entry.memory.saturating_add(process.memory.get());
        }
    }
    totals
}

fn usage_text(totals: Option<&Totals>) -> String {
    totals.map_or_else(
        || "no process visible".to_owned(),
        |total| {
            format!(
                "{} processes · cpu {}% · mem {}",
                total.processes,
                Permille::new(total.cpu),
                Bytes::new(total.memory)
            )
        },
    )
}

/// The Containers tab: what each runtime lists, with the processes found in each container.
///
/// # Arguments
///
/// * `listings` - what every installed runtime answered
/// * `usage` - CPU and memory of the running processes, matched to containers by their unit
#[must_use]
pub fn containers_view(listings: &[RuntimeListing], usage: &[ProcessUsage]) -> ContainersView {
    let totals = totals_by_container(usage);
    let mut view = ContainersView {
        has_runtime: !listings.is_empty(),
        ..ContainersView::default()
    };
    let mut listed_ids: Vec<&str> = Vec::new();

    for listing in listings {
        match listing.containers() {
            Ok(containers) => {
                for container in containers {
                    listed_ids.push(&container.id);
                    view.containers
                        .push(container_view(listing.runtime(), container, &totals));
                }
            }
            Err(error) => view
                .problems
                .push(format!("{}: {error}", listing.runtime())),
        }
    }
    view.unlisted = unlisted_scopes(&totals, &listed_ids);
    view
}

/// Container scopes that run processes but that no installed runtime listed (the runtime may be
/// missing, stopped, or refuse this user), biggest memory first.
fn unlisted_scopes(totals: &BTreeMap<&str, Totals>, listed_ids: &[&str]) -> Vec<String> {
    let mut unlisted: Vec<(&str, &Totals)> = totals
        .iter()
        .filter(|(full, _)| !listed_ids.iter().any(|short| same_container(full, short)))
        .map(|(id, total)| (*id, total))
        .collect();
    unlisted.sort_by_key(|(_, total)| Reverse(total.memory));
    unlisted
        .into_iter()
        .map(|(id, total)| {
            let short = id.get(..SHORT_ID_LEN).unwrap_or(id);
            format!("{short} · {}", usage_text(Some(total)))
        })
        .collect()
}

/// One listed container, with the CPU and memory of the processes found in its scope.
fn container_view(
    runtime: &str,
    container: &ContainerInfo,
    totals: &BTreeMap<&str, Totals>,
) -> ContainerView {
    let found = totals
        .iter()
        .find(|(full, _)| same_container(full, &container.id))
        .map(|(_, total)| total);
    ContainerView {
        runtime: runtime.to_owned(),
        id: container.id.clone(),
        name: container.name.clone(),
        image: container.image.clone(),
        status: container.status.clone(),
        running: container.is_running(),
        usage: usage_text(found),
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]
mod tests {
    use borbatop_core::{
        ContainerInfo, Pid, SocketProtocol, parse_socket_table, parse_startup_blame,
    };
    use borbatop_sources::{Connection, SocketOwner};

    use super::*;

    fn texts(report: &Report) -> Vec<&str> {
        report.lines.iter().map(|line| line.text.as_str()).collect()
    }

    fn tone_of(report: &Report, needle: &str) -> ReportTone {
        report
            .lines
            .iter()
            .find(|line| line.text.contains(needle))
            .unwrap_or_else(|| panic!("no line contains {needle:?}"))
            .tone
    }

    #[test]
    fn durations_read_like_systemd_prints_them() {
        assert_eq!(format_duration(Duration::from_millis(889)), "889ms");
        assert_eq!(format_duration(Duration::from_millis(5_240)), "5.2s");
        assert_eq!(format_duration(Duration::from_millis(63_300)), "1min 3.3s");
        assert_eq!(format_duration(Duration::from_secs(120)), "2min 0.0s");
    }

    #[test]
    fn boot_lists_slow_units_loudly_and_keeps_the_summary() {
        let blame = parse_startup_blame(
            "6.253s wait-online.service\n 1.2s mid.service\n 45ms tiny.service\n",
        );
        let report = boot_report(Ok("Startup finished in 24.2s\n"), Ok(&blame));

        assert_eq!(texts(&report)[0], "Startup");
        assert!(texts(&report).contains(&"  Startup finished in 24.2s"));
        assert_eq!(tone_of(&report, "wait-online"), ReportTone::Bad);
        assert_eq!(tone_of(&report, "mid.service"), ReportTone::Normal);
        assert_eq!(tone_of(&report, "tiny.service"), ReportTone::Dim);
    }

    #[test]
    fn boot_failures_are_red_lines_not_missing_sections() {
        let report = boot_report(Err("not found"), Err("not found"));
        assert_eq!(
            tone_of(&report, "systemd-analyze: not found"),
            ReportTone::Bad
        );
        assert_eq!(tone_of(&report, "blame: not found"), ReportTone::Bad);
    }

    #[test]
    fn logs_show_both_levels_and_empty_ones_as_good_news() {
        let report = logs_report(
            Ok(""),
            Ok("Nov 21 10:00:00 host kernel: something odd\n-- Boot abc --\n"),
        );
        assert_eq!(texts(&report)[0], "Errors since boot");
        assert_eq!(tone_of(&report, "none"), ReportTone::Good);
        assert_eq!(tone_of(&report, "something odd"), ReportTone::Normal);
        assert_eq!(tone_of(&report, "-- Boot"), ReportTone::Dim);

        let failed = logs_report(Err("no access"), Ok(""));
        assert_eq!(tone_of(&failed, "journalctl: no access"), ReportTone::Bad);
    }

    #[test]
    fn gpu_cards_show_readings_and_n_a_for_what_is_missing() {
        let card = GpuCard {
            name: "GeForce".to_owned(),
            vendor: GpuVendor::Nvidia,
            driver: Some("nvidia".to_owned()),
            utilization_percent: Some(28),
            memory_used: Some(Bytes::new(1_073_741_824)),
            memory_total: Some(Bytes::new(8_589_934_592)),
            temperature: None,
        };
        let mut lines = Vec::new();
        gpu_card_lines(&card, &mut lines);
        let report = Report { lines };
        assert_eq!(texts(&report)[0], "NVIDIA · GeForce · nvidia");
        assert!(texts(&report).iter().any(|text| text.contains("28%")));
        assert!(
            texts(&report)
                .iter()
                .any(|text| text.contains("1.0G / 8.0G (12.5%)"))
        );
        assert!(
            texts(&report)
                .iter()
                .any(|text| text.contains("temperature  n/a"))
        );
    }

    #[test]
    fn connections_list_listeners_first_with_their_owners() {
        let table = parse_socket_table(
            "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n   0: 0A00A8C0:C350 0101A8C0:01BB 01 0:0 0:0 0 1000 0 5151 2\n   1: 0100007F:1F90 00000000:0000 0A 0:0 0:0 0 1000 0 4242 1\n",
            SocketProtocol::Tcp4,
        );
        let [established, listener] = table.entries() else {
            return assert_eq!(table.entries().len(), 2);
        };
        let report = ConnectionReport::new(
            vec![
                Connection::new(established.clone(), Vec::new()),
                Connection::new(
                    listener.clone(),
                    vec![
                        SocketOwner::new(Pid::new(42), "server"),
                        SocketOwner::new(Pid::new(43), "worker"),
                    ],
                ),
            ],
            3,
            1,
        );

        let shown = connections_report(&report);

        assert_eq!(texts(&shown)[0], "2 sockets · 1 listening");
        let rows = texts(&shown);
        assert!(rows[2].contains("LISTEN") && rows[2].contains("127.0.0.1:8080"));
        assert!(rows[2].ends_with("42 server +1"), "{}", rows[2]);
        assert!(rows[3].contains("ESTAB") && rows[3].ends_with(NO_OWNER));
        assert_eq!(shown.lines[2].tone, ReportTone::Good);
        assert!(
            rows.iter()
                .any(|row| row.contains("3 processes cannot be inspected"))
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("1 socket tables could not be read"))
        );
    }

    fn container(id: &str, name: &str) -> ContainerInfo {
        ContainerInfo {
            id: id.to_owned(),
            name: name.to_owned(),
            image: "docker.io/library/nginx".to_owned(),
            status: "Up 3 hours".to_owned(),
            state: "running".to_owned(),
        }
    }

    fn usage(unit: &str, cpu: u64, memory: u64) -> ProcessUsage {
        ProcessUsage {
            unit: unit.to_owned(),
            cpu: Permille::new(cpu),
            memory: Bytes::new(memory),
        }
    }

    #[test]
    fn containers_total_the_processes_of_each_scope() {
        let listings = [RuntimeListing::new(
            "podman",
            Ok(vec![container("0a1b2c3d4e5f", "web")]),
        )];
        let processes = [
            usage("libpod-0a1b2c3d4e5f67890123.scope", 100, 1_048_576),
            usage("libpod-0a1b2c3d4e5f67890123.scope", 50, 1_048_576),
            usage("sshd.service", 10, 1),
            usage("docker-ffffffffffff00000000.scope", 5, 2_097_152),
        ];

        let view = containers_view(&listings, &processes);

        let [web] = view.containers.as_slice() else {
            return assert_eq!(view.containers.len(), 1);
        };
        assert_eq!((web.runtime.as_str(), web.name.as_str()), ("podman", "web"));
        assert!(web.running);
        assert!(web.usage.contains("2 processes"), "{}", web.usage);
        assert!(web.usage.contains("cpu 15.0%") && web.usage.contains("mem 2.0M"));
        assert_eq!(view.unlisted.len(), 1);
        assert!(view.unlisted[0].starts_with("ffffffffffff"));
        assert!(view.unlisted[0].contains("1 processes"));
        assert!(view.has_runtime);
    }

    #[test]
    fn a_stopped_container_has_no_processes_and_says_so() {
        let mut stopped = container("9e8d7c6b5a49", "old");
        stopped.state = "exited".to_owned();
        let view = containers_view(&[RuntimeListing::new("docker", Ok(vec![stopped]))], &[]);
        assert!(!view.containers[0].running);
        assert_eq!(view.containers[0].usage, "no process visible");
    }

    #[test]
    fn no_runtime_and_a_failing_runtime_are_told_apart() {
        let none = containers_view(&[], &[]);
        assert!(!none.has_runtime);
        assert_eq!(none.containers, []);

        let failing = containers_view(
            &[RuntimeListing::new(
                "docker",
                Err(borbatop_sources::SourceError::ProcessGone(Pid::new(1))),
            )],
            &[],
        );
        assert!(failing.has_runtime);
        assert_eq!(failing.problems.len(), 1);
        assert!(failing.problems[0].starts_with("docker: "));
    }
}
