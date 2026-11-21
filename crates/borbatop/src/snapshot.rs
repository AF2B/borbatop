//! One-shot mode: take a sample, print it as JSON, exit. Meant for scripts and cron jobs.

use std::error::Error;
use std::thread;

use borbatop_core::{Json, Pressure, PressureLine};
use borbatop_sources::{SystemCollector, list_units};
use borbatop_tui::{Model, ProcessRow};

use crate::config::Settings;
use crate::model::ModelBuilder;
use crate::run::process_name;

const OSRELEASE: &str = "/proc/sys/kernel/osrelease";

/// Whether command lines are part of the output.
///
/// Command lines often hold passwords and tokens, so they are left out unless asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandLines {
    /// Print only the program name of each process.
    Omit,
    /// Print the full command line as well.
    Include,
}

/// Takes two samples one interval apart (usage needs a delta) and returns the second as JSON.
///
/// # Arguments
///
/// * `settings` - sampling interval and history length
/// * `commands` - whether full command lines are included
///
/// # Errors
///
/// Returns the error of the first reading that fails.
pub fn capture(settings: Settings, commands: CommandLines) -> Result<Json, Box<dyn Error>> {
    let mut collector = SystemCollector::system()?;
    let page_size = SystemCollector::page_size()?;
    let kernel = std::fs::read_to_string(OSRELEASE)
        .map_or_else(|_| String::new(), |text| text.trim().to_owned());
    let mut builder = ModelBuilder::new(settings.history_len, page_size, kernel);
    builder.set_services(list_units().map_err(|error| error.to_string()));

    builder.build(&collector.collect()?);
    thread::sleep(settings.interval);
    let model = builder.build(&collector.collect()?);

    Ok(model_json(&model, commands))
}

/// The JSON form of one sample.
///
/// # Arguments
///
/// * `model` - the sample
/// * `commands` - whether full command lines are included
#[must_use]
pub fn model_json(model: &Model, commands: CommandLines) -> Json {
    Json::object([
        ("kernel", Json::text(&model.kernel_release)),
        ("uptime_seconds", seconds(model)),
        ("load", load_json(model)),
        ("cores_permille", permilles(model)),
        ("memory", memory_json(model)),
        ("temperature_celsius", temperature_json(model)),
        ("pressure", pressure_json(model)),
        (
            "processes",
            Json::Array(
                model
                    .processes
                    .iter()
                    .map(|process| process_json(process, commands))
                    .collect(),
            ),
        ),
        ("services", services_json(model)),
        ("network", network_json(model)),
        ("disks", disks_json(model)),
        ("mounts", mounts_json(model)),
    ])
}

fn seconds(model: &Model) -> Json {
    Json::or_null(model.uptime.map(|uptime| Json::Unsigned(uptime.as_secs())))
}

fn load_json(model: &Model) -> Json {
    Json::or_null(model.load.map(|load| {
        Json::object([
            ("one", Json::Hundredths(load.one().get())),
            ("five", Json::Hundredths(load.five().get())),
            ("fifteen", Json::Hundredths(load.fifteen().get())),
        ])
    }))
}

fn permilles(model: &Model) -> Json {
    Json::Array(
        model
            .cores
            .iter()
            .map(|core| Json::Unsigned(core.get()))
            .collect(),
    )
}

fn memory_json(model: &Model) -> Json {
    Json::or_null(model.memory.map(|memory| {
        Json::object([
            ("total_bytes", Json::Unsigned(memory.total().get())),
            ("used_bytes", Json::Unsigned(memory.used().get())),
            ("available_bytes", Json::Unsigned(memory.available().get())),
            (
                "swap_total_bytes",
                Json::Unsigned(memory.swap_total().get()),
            ),
            ("swap_used_bytes", Json::Unsigned(memory.swap_used().get())),
        ])
    }))
}

fn temperature_json(model: &Model) -> Json {
    Json::or_null(
        model
            .temperature
            .map(|degrees| Json::Signed(i64::from(degrees.rounded_degrees()))),
    )
}

fn pressure_json(model: &Model) -> Json {
    let resource = |pressure: Option<Pressure>| {
        Json::or_null(pressure.map(|pressure| {
            Json::object([
                ("some", pressure_line_json(pressure.some())),
                (
                    "full",
                    Json::or_null(pressure.full().map(pressure_line_json)),
                ),
            ])
        }))
    };
    Json::object([
        ("cpu", resource(model.pressure.cpu)),
        ("memory", resource(model.pressure.memory)),
        ("io", resource(model.pressure.io)),
    ])
}

fn pressure_line_json(line: PressureLine) -> Json {
    Json::object([
        ("avg10", Json::Hundredths(line.avg10().get())),
        ("avg60", Json::Hundredths(line.avg60().get())),
        ("avg300", Json::Hundredths(line.avg300().get())),
    ])
}

fn process_json(process: &ProcessRow, commands: CommandLines) -> Json {
    let mut fields = vec![
        (
            "pid".to_owned(),
            Json::Unsigned(u64::from(process.pid.get())),
        ),
        (
            "parent".to_owned(),
            Json::Unsigned(u64::from(process.parent.get())),
        ),
        ("user".to_owned(), Json::text(&process.user)),
        (
            "state".to_owned(),
            Json::Text(process.state.code().to_string()),
        ),
        ("cpu_permille".to_owned(), Json::Unsigned(process.cpu.get())),
        (
            "memory_bytes".to_owned(),
            Json::Unsigned(process.memory.get()),
        ),
        (
            "threads".to_owned(),
            Json::Unsigned(u64::from(process.threads.get())),
        ),
        ("unit".to_owned(), Json::text(&process.unit)),
        ("package".to_owned(), Json::text(&process.package)),
        (
            "name".to_owned(),
            Json::Text(process_name(&process.command)),
        ),
    ];
    match commands {
        CommandLines::Omit => {}
        CommandLines::Include => {
            fields.push(("command".to_owned(), Json::text(&process.command)));
        }
    }
    Json::Object(fields)
}

fn services_json(model: &Model) -> Json {
    match &model.services {
        Ok(list) => Json::Array(
            list.units()
                .iter()
                .map(|unit| {
                    Json::object([
                        ("name", Json::text(unit.name())),
                        ("active", Json::text(unit.active().name())),
                        ("sub", Json::text(unit.sub())),
                    ])
                })
                .collect(),
        ),
        Err(_reason) => Json::Null,
    }
}

fn network_json(model: &Model) -> Json {
    Json::Array(
        model
            .network
            .iter()
            .map(|interface| {
                Json::object([
                    ("name", Json::text(&interface.name)),
                    (
                        "received_bytes_per_second",
                        Json::or_null(interface.received.map(|rate| Json::Unsigned(rate.get()))),
                    ),
                    (
                        "sent_bytes_per_second",
                        Json::or_null(interface.sent.map(|rate| Json::Unsigned(rate.get()))),
                    ),
                ])
            })
            .collect(),
    )
}

fn disks_json(model: &Model) -> Json {
    Json::Array(
        model
            .disks
            .iter()
            .map(|disk| {
                Json::object([
                    ("name", Json::text(&disk.name)),
                    (
                        "read_bytes_per_second",
                        Json::or_null(disk.read.map(|rate| Json::Unsigned(rate.get()))),
                    ),
                    (
                        "written_bytes_per_second",
                        Json::or_null(disk.written.map(|rate| Json::Unsigned(rate.get()))),
                    ),
                ])
            })
            .collect(),
    )
}

fn mounts_json(model: &Model) -> Json {
    Json::Array(
        model
            .mounts
            .iter()
            .map(|mount| {
                Json::object([
                    ("mount_point", Json::text(&mount.mount_point)),
                    ("fs_type", Json::text(&mount.fs_type)),
                    ("total_bytes", Json::Unsigned(mount.total.get())),
                    ("used_bytes", Json::Unsigned(mount.used.get())),
                ])
            })
            .collect(),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::time::Duration;

    use borbatop_core::{Bytes, Permille, ProcessState, parse_process_stat};

    use super::*;

    fn process(command: &str) -> ProcessRow {
        let stat = "42 (x) S 1 1 1 0 -1 0 0 0 0 0 0 0 0 0 20 0 3 0 9 1 1 0";
        let sample = parse_process_stat(stat).unwrap();
        ProcessRow {
            pid: sample.pid(),
            start_time: sample.id().start_time(),
            parent: sample.parent(),
            user: "borba".to_owned(),
            state: ProcessState::Sleeping,
            cpu: Permille::new(250),
            memory: Bytes::new(4_096),
            threads: sample.threads(),
            unit: "app.scope".to_owned(),
            package: "pkg".to_owned(),
            command: command.to_owned(),
        }
    }

    fn model_with(rows: Vec<ProcessRow>) -> Model {
        let mut model = Model::empty(4);
        model.kernel_release = "7.2.1-arch1".to_owned();
        model.uptime = Some(Duration::from_secs(90));
        model.processes = rows;
        model
    }

    #[test]
    fn command_lines_are_omitted_unless_requested() {
        let model = model_with(vec![process("/usr/bin/tool --token=secret")]);

        let omitted = model_json(&model, CommandLines::Omit).render();
        assert!(!omitted.contains("secret"));
        assert!(omitted.contains(r#""name":"tool""#));

        let included = model_json(&model, CommandLines::Include).render();
        assert!(included.contains("--token=secret"));
    }

    #[test]
    fn top_level_fields_and_nulls_for_missing_readings() {
        let text = model_json(&model_with(Vec::new()), CommandLines::Omit).render();
        assert!(text.starts_with(r#"{"kernel":"7.2.1-arch1","uptime_seconds":90,"load":null,"#));
        assert!(text.contains(r#""memory":null"#));
        assert!(text.contains(r#""services":null"#));
        assert!(text.contains(r#""processes":[]"#));
    }

    #[test]
    fn process_fields_use_plain_units() {
        let text = model_json(&model_with(vec![process("tool")]), CommandLines::Omit).render();
        assert!(text.contains(r#""pid":42"#));
        assert!(text.contains(r#""cpu_permille":250"#));
        assert!(text.contains(r#""memory_bytes":4096"#));
        assert!(text.contains(r#""state":"S""#));
    }

    #[test]
    fn hostile_text_is_escaped() {
        let mut row = process("tool");
        row.user = "a\"b\u{1b}".to_owned();
        let text = model_json(&model_with(vec![row]), CommandLines::Omit).render();
        assert!(text.contains(r#""user":"a\"b\u001b""#));
        assert!(!text.chars().any(|character| character == '\u{1b}'));
    }
}
