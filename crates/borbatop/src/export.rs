//! Writing the tab being shown to a JSON file, exactly as the person was seeing it.
//!
//! The file lands in the directory where the program was started, readable by its owner only,
//! and an existing file is never overwritten. Command lines are never written: they may carry
//! secrets.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use borbatop_core::{Json, UnitRow, UtcDateTime};
use borbatop_tui::{
    AurView, ContainersView, ExportContent, ExportOrder, ExportRequest, KernelView, Model,
    PacmanView, RepoSource, Report, StaleView,
};

use crate::snapshot::{
    CommandLines, disks_json, mounts_json, network_json, process_json, unit_json,
};

const FILE_MODE: u32 = 0o600;
const NAME_ATTEMPTS: u32 = 10;

/// Why an export could not be written.
#[derive(Debug)]
pub struct ExportError {
    path: PathBuf,
    source: io::Error,
}

impl fmt::Display for ExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.path.display(), self.source)
    }
}

impl Error for ExportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

/// Writes the export into `directory` and returns the path of the new file.
///
/// The name is `borbatop-<tab>-<YYYYMMDD-HHMMSS>.json` (UTC); when that name is taken, a numeric
/// suffix is added, so two exports in the same second never overwrite each other.
///
/// # Arguments
///
/// * `directory` - where to create the file
/// * `request` - what the person was looking at
/// * `model` - the sample the request refers to
/// * `now` - the time to stamp the file with
///
/// # Errors
///
/// Returns an [`ExportError`] naming the file when it cannot be created or written.
pub fn write_export(
    directory: &Path,
    request: &ExportRequest,
    model: &Model,
    now: SystemTime,
) -> Result<PathBuf, ExportError> {
    let seconds = now
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let moment = UtcDateTime::from_unix_seconds(seconds);
    let text = export_json(request, model, &moment).render();

    for attempt in 0..NAME_ATTEMPTS {
        let suffix = if attempt == 0 {
            String::new()
        } else {
            format!("-{attempt}")
        };
        let path = directory.join(format!(
            "borbatop-{}-{}{suffix}.json",
            request.tab.slug(),
            moment.file_stamp()
        ));
        let opened = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(FILE_MODE)
            .open(&path);
        match opened {
            Ok(mut file) => {
                writeln!(file, "{text}").map_err(|source| ExportError {
                    path: path.clone(),
                    source,
                })?;
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(source) => return Err(ExportError { path, source }),
        }
    }
    Err(ExportError {
        path: directory.to_path_buf(),
        source: io::Error::new(
            io::ErrorKind::AlreadyExists,
            "too many exports in one second",
        ),
    })
}

/// The JSON document of one export.
///
/// # Arguments
///
/// * `request` - what the person was looking at
/// * `model` - the sample the request refers to
/// * `moment` - when the export was made
#[must_use]
pub fn export_json(request: &ExportRequest, model: &Model, moment: &UtcDateTime) -> Json {
    Json::object([
        ("tab", Json::text(request.tab.slug())),
        ("exported_at", Json::Text(moment.to_string())),
        ("filter", Json::text(&request.filter)),
        ("order", order_json(request.order)),
        ("data", content_json(&request.content, model)),
    ])
}

fn order_json(order: Option<ExportOrder>) -> Json {
    Json::or_null(order.map(|order| {
        Json::object([
            ("column", Json::text(order.column.label())),
            ("descending", Json::Bool(order.descending)),
            ("tree", Json::Bool(order.tree)),
        ])
    }))
}

fn content_json(content: &ExportContent, model: &Model) -> Json {
    match content {
        ExportContent::Processes(pids) => {
            let rows: HashMap<_, _> = model
                .processes
                .iter()
                .map(|process| (process.pid, process))
                .collect();
            Json::Array(
                pids.iter()
                    .filter_map(|pid| rows.get(pid))
                    .map(|process| process_json(process, CommandLines::Omit))
                    .collect(),
            )
        }
        ExportContent::Services(names) => services(names, model),
        ExportContent::Io => Json::object([
            ("network", network_json(model)),
            ("disks", disks_json(model)),
            ("mounts", mounts_json(model)),
        ]),
        ExportContent::Pacman(view) => pacman_json(view),
        ExportContent::Containers(view) => containers_json(view),
        ExportContent::Report(report) => report_json(report),
    }
}

fn services(names: &[String], model: &Model) -> Json {
    let Ok(list) = &model.services else {
        return Json::Array(Vec::new());
    };
    let units: HashMap<&str, &UnitRow> = list
        .units()
        .iter()
        .map(|unit| (unit.name(), unit))
        .collect();
    Json::Array(
        names
            .iter()
            .filter_map(|name| units.get(name.as_str()))
            .map(|unit| unit_json(unit))
            .collect(),
    )
}

fn containers_json(view: &ContainersView) -> Json {
    Json::object([
        (
            "containers",
            Json::Array(
                view.containers
                    .iter()
                    .map(|container| {
                        Json::object([
                            ("runtime", Json::text(&container.runtime)),
                            ("id", Json::text(&container.id)),
                            ("name", Json::text(&container.name)),
                            ("image", Json::text(&container.image)),
                            ("status", Json::text(&container.status)),
                            ("running", Json::Bool(container.running)),
                            ("usage", Json::text(&container.usage)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "problems",
            Json::Array(view.problems.iter().map(|text| Json::text(text)).collect()),
        ),
        (
            "unlisted_scopes",
            Json::Array(view.unlisted.iter().map(|text| Json::text(text)).collect()),
        ),
    ])
}

fn report_json(report: &Report) -> Json {
    Json::Array(
        report
            .lines
            .iter()
            .map(|line| {
                Json::object([
                    (
                        "tone",
                        Json::Text(format!("{:?}", line.tone).to_lowercase()),
                    ),
                    ("text", Json::text(&line.text)),
                ])
            })
            .collect(),
    )
}

/// A part of the Pacman report, or the reason it is unknown.
fn part<T>(result: &Result<T, String>, convert: impl FnOnce(&T) -> Json) -> Json {
    match result {
        Ok(value) => convert(value),
        Err(reason) => Json::object([("error", Json::text(reason))]),
    }
}

fn upgrades_json(upgrades: &[borbatop_core::Upgrade]) -> Json {
    Json::Array(
        upgrades
            .iter()
            .map(|upgrade| {
                Json::object([
                    ("name", Json::text(upgrade.name())),
                    ("from", Json::text(upgrade.from())),
                    ("to", Json::text(upgrade.to())),
                ])
            })
            .collect(),
    )
}

fn aur_json(aur: &[AurView]) -> Json {
    Json::Array(
        aur.iter()
            .map(|found| {
                Json::object([
                    ("helper", Json::text(&found.helper)),
                    (
                        "upgrades",
                        part(&found.upgrades, |list| upgrades_json(list)),
                    ),
                ])
            })
            .collect(),
    )
}

fn kernel_json(kernel: &KernelView) -> Json {
    Json::object([
        ("running", Json::text(&kernel.running)),
        ("reboot_recommended", Json::Bool(kernel.reboot_recommended)),
    ])
}

fn stale_json(stale: &[StaleView]) -> Json {
    Json::Array(
        stale
            .iter()
            .map(|process| {
                Json::object([
                    ("pid", Json::Unsigned(u64::from(process.pid.get()))),
                    ("comm", Json::text(&process.comm)),
                    ("library", Json::text(&process.library)),
                ])
            })
            .collect(),
    )
}

fn pacman_json(view: &PacmanView) -> Json {
    Json::object([
        ("upgrades", part(&view.upgrades, |list| upgrades_json(list))),
        (
            "upgrades_source",
            Json::text(match view.repo_source {
                RepoSource::Checkupdates => "checkupdates",
                RepoSource::LocalDatabase => "local database",
            }),
        ),
        ("aur", aur_json(&view.aur)),
        (
            "orphans",
            part(&view.orphans, |names| {
                Json::Array(names.iter().map(|name| Json::text(name)).collect())
            }),
        ),
        ("kernel", part(&view.kernel, kernel_json)),
        (
            "pending_config",
            part(&view.pending_config, |files| {
                Json::Array(files.iter().map(|file| Json::text(file)).collect())
            }),
        ),
        (
            "cache_bytes",
            part(&view.cache, |size| Json::Unsigned(size.get())),
        ),
        ("stale_libraries", stale_json(&view.stale)),
        (
            "stale_unreadable_processes",
            Json::Unsigned(u64::try_from(view.stale_unreadable).unwrap_or(u64::MAX)),
        ),
    ])
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;

    use borbatop_core::{
        Bytes, Permille, Pid, ProcessState, parse_process_stat, parse_systemctl_units,
    };
    use borbatop_tui::{ProcessRow, ReportLine, ReportTone, SortKey, Tab};

    use super::*;

    fn process(pid: u32, command: &str) -> ProcessRow {
        let stat = format!("{pid} (x) S 1 1 1 0 -1 0 0 0 0 0 0 0 0 0 20 0 3 0 9 1 1 0");
        let parsed = parse_process_stat(&stat).unwrap();
        ProcessRow {
            pid: parsed.pid(),
            start_time: parsed.id().start_time(),
            parent: parsed.parent(),
            user: "borba".to_owned(),
            state: ProcessState::Sleeping,
            cpu: Permille::new(10),
            memory: Bytes::new(2_048),
            nice: parsed.nice(),
            threads: parsed.threads(),
            unit: "app.scope".to_owned(),
            package: "pkg".to_owned(),
            command: command.to_owned(),
        }
    }

    fn model() -> Model {
        let mut model = Model::empty(4);
        model.processes = vec![
            process(1, "/usr/bin/one --token=secret"),
            process(2, "two"),
            process(3, "three"),
        ];
        model.services = Ok(parse_systemctl_units(
            "a.service loaded active running A\nb.service loaded failed failed B\nc.timer loaded active waiting C\n",
        ));
        model
    }

    fn request(content: ExportContent) -> ExportRequest {
        ExportRequest {
            tab: Tab::Processes,
            filter: "tw".to_owned(),
            order: Some(ExportOrder {
                column: SortKey::Memory,
                descending: true,
                tree: false,
            }),
            content,
        }
    }

    fn moment() -> UtcDateTime {
        UtcDateTime::from_unix_seconds(1_000_000_000)
    }

    fn pids_of(json: &Json) -> Vec<u64> {
        json.get("data")
            .and_then(Json::as_array)
            .unwrap()
            .iter()
            .map(|row| row.get("pid").and_then(Json::as_u64).unwrap())
            .collect()
    }

    #[test]
    fn processes_come_out_in_the_order_and_selection_the_person_saw() {
        let shown = request(ExportContent::Processes(vec![
            Pid::new(3),
            Pid::new(1),
            Pid::new(99),
        ]));

        let json = export_json(&shown, &model(), &moment());

        assert_eq!(pids_of(&json), [3, 1], "pid 99 vanished and is skipped");
        assert_eq!(json.get("filter").and_then(Json::as_str), Some("tw"));
        let order = json.get("order").unwrap();
        assert_eq!(order.get("column").and_then(Json::as_str), Some("MEM"));
        assert!(matches!(order.get("descending"), Some(Json::Bool(true))));
        assert_eq!(
            json.get("exported_at").and_then(Json::as_str),
            Some("2001-09-09 01:46:40 UTC")
        );
    }

    #[test]
    fn command_lines_never_reach_the_file() {
        let shown = request(ExportContent::Processes(vec![Pid::new(1)]));
        let text = export_json(&shown, &model(), &moment()).render();
        assert!(!text.contains("secret"), "{text}");
        assert!(text.contains(r#""name":"one""#));
    }

    #[test]
    fn services_keep_the_filtered_order() {
        let mut shown = request(ExportContent::Services(vec![
            "c.timer".to_owned(),
            "a.service".to_owned(),
        ]));
        shown.tab = Tab::Services;
        shown.order = None;

        let json = export_json(&shown, &model(), &moment());

        let names: Vec<&str> = json
            .get("data")
            .and_then(Json::as_array)
            .unwrap()
            .iter()
            .map(|unit| unit.get("name").and_then(Json::as_str).unwrap())
            .collect();
        assert_eq!(names, ["c.timer", "a.service"]);
        assert!(json.get("order").unwrap().is_null());
    }

    #[test]
    fn reports_keep_their_lines_and_tones() {
        let mut shown = request(ExportContent::Report(Report {
            lines: vec![
                ReportLine::new("Errors since boot", ReportTone::Heading),
                ReportLine::new("kernel: bad", ReportTone::Bad),
            ],
        }));
        shown.tab = Tab::Logs;

        let text = export_json(&shown, &model(), &moment()).render();

        assert!(
            text.contains(r#""tone":"heading","text":"Errors since boot""#),
            "{text}"
        );
        assert!(text.contains(r#""tone":"bad","text":"kernel: bad""#));
    }

    #[test]
    fn containers_export_their_state_and_problems() {
        let view = ContainersView {
            containers: vec![borbatop_tui::ContainerView {
                runtime: "podman".to_owned(),
                id: "0a1b2c3d4e5f".to_owned(),
                name: "web".to_owned(),
                image: "nginx".to_owned(),
                status: "Up 3 hours".to_owned(),
                running: true,
                usage: "3 processes".to_owned(),
            }],
            problems: vec!["docker: permission denied".to_owned()],
            has_runtime: true,
            unlisted: vec!["ffffffffffff · 1 processes".to_owned()],
        };
        let mut shown = request(ExportContent::Containers(Box::new(view)));
        shown.tab = Tab::Containers;

        let text = export_json(&shown, &model(), &moment()).render();

        assert!(
            text.contains(r#""runtime":"podman","id":"0a1b2c3d4e5f","name":"web""#),
            "{text}"
        );
        assert!(text.contains(r#""running":true"#));
        assert!(text.contains(r#""problems":["docker: permission denied"]"#));
        assert!(text.contains(r#""unlisted_scopes":["ffffffffffff · 1 processes"]"#));
    }

    #[test]
    fn a_pacman_part_that_failed_becomes_an_error_object() {
        let view = PacmanView {
            upgrades: Ok(Vec::new()),
            repo_source: RepoSource::LocalDatabase,
            aur: Vec::new(),
            orphans: Err("pacman -Qdtq: not found".to_owned()),
            kernel: Ok(KernelView {
                running: "7.2.1".to_owned(),
                reboot_recommended: false,
            }),
            pending_config: Ok(Vec::new()),
            cache: Ok(Bytes::new(4_096)),
            stale: Vec::new(),
            stale_unreadable: 2,
        };
        let mut shown = request(ExportContent::Pacman(Box::new(view)));
        shown.tab = Tab::Pacman;

        let text = export_json(&shown, &model(), &moment()).render();

        assert!(
            text.contains(r#""orphans":{"error":"pacman -Qdtq: not found"}"#),
            "{text}"
        );
        assert!(text.contains(r#""aur":[]"#));
        assert!(text.contains(r#""upgrades_source":"local database""#));
        assert!(text.contains(r#""cache_bytes":4096"#));
        assert!(text.contains(r#""stale_unreadable_processes":2"#));
    }

    fn temp_directory(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("borbatop-export-{name}-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn the_file_is_private_stamped_and_valid_json() {
        let directory = temp_directory("file");
        let at = UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        let shown = request(ExportContent::Processes(vec![Pid::new(2)]));

        let path = write_export(&directory, &shown, &model(), at).unwrap();

        assert_eq!(
            path.file_name().unwrap(),
            "borbatop-processes-20010909-014640.json"
        );
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let parsed = Json::parse(fs::read_to_string(&path).unwrap().trim()).unwrap();
        assert_eq!(pids_of(&parsed), [2]);
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn exporting_twice_in_one_second_never_overwrites() {
        let directory = temp_directory("twice");
        let at = UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        let shown = request(ExportContent::Processes(vec![Pid::new(2)]));

        let first = write_export(&directory, &shown, &model(), at).unwrap();
        let second = write_export(&directory, &shown, &model(), at).unwrap();

        assert_ne!(first, second);
        assert!(second.to_string_lossy().ends_with("-1.json"));
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 2);
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn an_unwritable_directory_is_reported_with_its_path() {
        let shown = request(ExportContent::Io);
        let error = write_export(
            Path::new("/nonexistent-directory"),
            &shown,
            &model(),
            UNIX_EPOCH,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("/nonexistent-directory"),
            "{error}"
        );
    }
}
