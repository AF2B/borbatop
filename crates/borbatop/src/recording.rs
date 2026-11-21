//! Recording samples to a file and loading them back for replay.
//!
//! A recording is JSON Lines: one snapshot (see [`crate::snapshot`]) per line. It is meant to be
//! read by people (`jq`) as well as by `borbatop replay`.
//!
//! A recording lists every process and user on the machine, and command lines when asked for, so
//! the file is created readable by its owner only and an existing file is never overwritten.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use borbatop_core::{
    ActiveState, Bytes, BytesPerSecond, History, Hundredths, Json, JsonError, LoadAvg, MemInfo,
    MilliCelsius, Nice, Permille, Pid, Pressure, PressureLine, ProcessState, StartTime,
    ThreadCount, UnitList, UnitRow,
};
use borbatop_tui::{DiskRow, Model, MountRow, NetRow, PressureView, ProcessRow};

use crate::config::Settings;
use crate::snapshot::{CommandLines, LiveSampler, model_json};

const FILE_MODE: u32 = 0o600;
const MILLI_PER_DEGREE: i64 = 1_000;

/// Why a recording could not be written or read.
#[derive(Debug)]
pub enum RecordingError {
    /// The file could not be created, written or read.
    Io {
        /// File that failed.
        path: PathBuf,
        /// Underlying OS error.
        source: io::Error,
    },
    /// A line is not valid JSON.
    Json {
        /// One-based line number.
        line: usize,
        /// Why it was rejected.
        source: JsonError,
    },
    /// A line is valid JSON but lacks a field, or the field has the wrong type.
    Field {
        /// One-based line number.
        line: usize,
        /// Name of the field.
        field: &'static str,
    },
    /// The file holds no sample at all.
    Empty {
        /// File that was read.
        path: PathBuf,
    },
}

impl fmt::Display for RecordingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::Json { line, source } => write!(formatter, "line {line}: {source}"),
            Self::Field { line, field } => {
                write!(formatter, "line {line}: missing or invalid field `{field}`")
            }
            Self::Empty { path } => write!(formatter, "{}: no samples", path.display()),
        }
    }
}

impl Error for RecordingError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json { source, .. } => Some(source),
            Self::Field { .. } | Self::Empty { .. } => None,
        }
    }
}

/// Records `samples` samples, one per interval, to a new file.
///
/// The first reading only primes the usage counters and is not written, so every recorded
/// sample has real CPU and throughput figures.
///
/// # Arguments
///
/// * `settings` - sampling interval and history length
/// * `path` - file to create; it must not exist
/// * `samples` - how many samples to write
/// * `commands` - whether full command lines are written (they may contain secrets)
///
/// # Errors
///
/// Returns [`RecordingError::Io`] when the file exists or cannot be written, or the error of a
/// reading that fails.
pub fn record(
    settings: Settings,
    path: &Path,
    samples: u32,
    commands: CommandLines,
) -> Result<(), Box<dyn Error>> {
    let io_error = |source| RecordingError::Io {
        path: path.to_path_buf(),
        source,
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(FILE_MODE)
        .open(path)
        .map_err(io_error)?;

    let mut machine = LiveSampler::new(settings)?;
    machine.sample()?;
    for _ in 0..samples {
        thread::sleep(settings.interval);
        let line = model_json(&machine.sample()?, commands).render();
        writeln!(file, "{line}").map_err(io_error)?;
        file.flush().map_err(io_error)?;
    }
    Ok(())
}

/// Loads a recording as one model per sample, with sparkline histories rebuilt along the way.
///
/// A last line that is cut off (the recording was interrupted) is ignored.
///
/// # Arguments
///
/// * `path` - recording to read
/// * `history_len` - samples kept for sparklines
///
/// # Errors
///
/// Returns a [`RecordingError`] naming the file, line and field of the first problem.
pub fn load(path: &Path, history_len: usize) -> Result<Vec<Model>, RecordingError> {
    let text = fs::read_to_string(path).map_err(|source| RecordingError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let lines: Vec<(usize, &str)> = text
        .lines()
        .enumerate()
        .map(|(index, line)| (index.saturating_add(1), line))
        .filter(|(_, line)| !line.trim().is_empty())
        .collect();

    let mut models = Vec::with_capacity(lines.len());
    let mut histories = Histories::new(history_len);
    for (position, &(number, line)) in lines.iter().enumerate() {
        let json = match Json::parse(line) {
            Ok(json) => json,
            Err(_) if position.saturating_add(1) == lines.len() && !models.is_empty() => break,
            Err(source) => {
                return Err(RecordingError::Json {
                    line: number,
                    source,
                });
            }
        };
        let mut model =
            model_from_json(&json, history_len).map_err(|field| RecordingError::Field {
                line: number,
                field,
            })?;
        histories.apply(&json, &mut model);
        models.push(model);
    }
    if models.is_empty() {
        return Err(RecordingError::Empty {
            path: path.to_path_buf(),
        });
    }
    Ok(models)
}

/// Sparkline histories, rebuilt from the samples as they are loaded in order.
struct Histories {
    len: usize,
    cpu: History<u64>,
    network: HashMap<String, (History<u64>, History<u64>)>,
}

impl Histories {
    fn new(len: usize) -> Self {
        Self {
            len,
            cpu: History::new(len),
            network: HashMap::new(),
        }
    }

    fn apply(&mut self, json: &Json, model: &mut Model) {
        let total = json.get("cpu_permille").and_then(Json::as_u64).unwrap_or(0);
        self.cpu.push(total);
        model.cpu_history = self.cpu.clone();

        for row in &mut model.network {
            let len = self.len;
            let (received, sent) = self
                .network
                .entry(row.name.clone())
                .or_insert_with(|| (History::new(len), History::new(len)));
            received.push(row.received.map_or(0, BytesPerSecond::get));
            sent.push(row.sent.map_or(0, BytesPerSecond::get));
            row.received_history = received.clone();
            row.sent_history = sent.clone();
        }
    }
}

type Field<T> = Result<T, &'static str>;

fn member<'a>(json: &'a Json, key: &'static str) -> Field<&'a Json> {
    json.get(key).ok_or(key)
}

fn unsigned(json: &Json, key: &'static str) -> Field<u64> {
    member(json, key)?.as_u64().ok_or(key)
}

fn narrow<T: TryFrom<u64>>(json: &Json, key: &'static str) -> Field<T> {
    T::try_from(unsigned(json, key)?).map_err(|_| key)
}

fn text<'a>(json: &'a Json, key: &'static str) -> Field<&'a str> {
    member(json, key)?.as_str().ok_or(key)
}

fn hundredths(json: &Json, key: &'static str) -> Field<Hundredths> {
    member(json, key)?
        .as_hundredths()
        .map(Hundredths::new)
        .ok_or(key)
}

fn items<'a>(json: &'a Json, key: &'static str) -> Field<&'a [Json]> {
    member(json, key)?.as_array().ok_or(key)
}

/// A member that may be `null`.
fn optional<'a>(json: &'a Json, key: &'static str) -> Field<Option<&'a Json>> {
    let value = member(json, key)?;
    Ok((!value.is_null()).then_some(value))
}

fn rate(json: &Json, key: &'static str) -> Field<Option<BytesPerSecond>> {
    optional(json, key)?
        .map(|value| value.as_u64().map(BytesPerSecond::new).ok_or(key))
        .transpose()
}

fn pressure_line(json: &Json) -> Field<PressureLine> {
    Ok(PressureLine::new(
        hundredths(json, "avg10")?,
        hundredths(json, "avg60")?,
        hundredths(json, "avg300")?,
    ))
}

fn pressure(json: &Json, key: &'static str) -> Field<Option<Pressure>> {
    let Some(value) = optional(json, key)? else {
        return Ok(None);
    };
    let some = pressure_line(member(value, "some")?)?;
    let full = optional(value, "full")?.map(pressure_line).transpose()?;
    Ok(Some(Pressure::new(some, full)))
}

fn memory(json: &Json) -> Field<Option<MemInfo>> {
    let Some(value) = optional(json, "memory")? else {
        return Ok(None);
    };
    let swap_total = unsigned(value, "swap_total_bytes")?;
    let swap_used = unsigned(value, "swap_used_bytes")?;
    Ok(Some(MemInfo::new(
        Bytes::new(unsigned(value, "total_bytes")?),
        Bytes::new(unsigned(value, "available_bytes")?),
        Bytes::new(swap_total),
        Bytes::new(swap_total.saturating_sub(swap_used)),
    )))
}

fn process(json: &Json) -> Field<ProcessRow> {
    let name = text(json, "name")?;
    let nice = member(json, "nice")?
        .as_i64()
        .and_then(|value| i8::try_from(value).ok())
        .ok_or("nice")?;
    let state = text(json, "state")?
        .chars()
        .next()
        .and_then(|code| ProcessState::from_code(code).ok())
        .ok_or("state")?;
    Ok(ProcessRow {
        pid: Pid::new(narrow(json, "pid")?),
        start_time: StartTime::new(unsigned(json, "start_time")?),
        parent: Pid::new(narrow(json, "parent")?),
        user: text(json, "user")?.to_owned(),
        state,
        cpu: Permille::new(unsigned(json, "cpu_permille")?),
        memory: Bytes::new(unsigned(json, "memory_bytes")?),
        nice: Nice::clamped(nice),
        threads: ThreadCount::new(narrow(json, "threads")?),
        unit: text(json, "unit")?.to_owned(),
        package: text(json, "package")?.to_owned(),
        // Without recorded command lines the program name is all there is to show.
        command: json
            .get("command")
            .and_then(Json::as_str)
            .unwrap_or(name)
            .to_owned(),
    })
}

fn services(json: &Json) -> Field<Result<UnitList, String>> {
    let Some(value) = optional(json, "services")? else {
        return Ok(Err("systemd units were not recorded".to_owned()));
    };
    let units = value
        .as_array()
        .ok_or("services")?
        .iter()
        .map(|unit| {
            let active = ActiveState::from_name(text(unit, "active")?).ok_or("active")?;
            let description = unit.get("description").and_then(Json::as_str).unwrap_or("");
            Ok(UnitRow::new(
                text(unit, "name")?,
                active,
                text(unit, "sub")?,
                description,
            ))
        })
        .collect::<Field<Vec<UnitRow>>>()?;
    Ok(Ok(UnitList::new(units, 0)))
}

fn temperature(json: &Json) -> Field<Option<MilliCelsius>> {
    optional(json, "temperature_celsius")?
        .map(|value| {
            let degrees = value.as_i64().ok_or("temperature_celsius")?;
            let milli = degrees
                .checked_mul(MILLI_PER_DEGREE)
                .and_then(|milli| i32::try_from(milli).ok())
                .ok_or("temperature_celsius")?;
            Ok(MilliCelsius::new(milli))
        })
        .transpose()
}

/// Rebuilds a [`Model`] from one snapshot line; histories are filled in by the caller.
fn model_from_json(json: &Json, history_len: usize) -> Field<Model> {
    let mut model = Model::empty(history_len);
    machine_fields(json, &mut model)?;
    table_fields(json, &mut model, history_len)?;
    Ok(model)
}

/// The machine-wide readings: kernel, uptime, load, cores, memory, temperature and pressure.
fn machine_fields(json: &Json, model: &mut Model) -> Field<()> {
    text(json, "kernel")?.clone_into(&mut model.kernel_release);
    model.uptime = optional(json, "uptime_seconds")?
        .map(|value| {
            value
                .as_u64()
                .map(Duration::from_secs)
                .ok_or("uptime_seconds")
        })
        .transpose()?;
    model.load = optional(json, "load")?
        .map(|load| {
            Ok(LoadAvg::new(
                hundredths(load, "one")?,
                hundredths(load, "five")?,
                hundredths(load, "fifteen")?,
            ))
        })
        .transpose()?;
    model.cores = items(json, "cores_permille")?
        .iter()
        .map(|core| core.as_u64().map(Permille::new).ok_or("cores_permille"))
        .collect::<Field<Vec<Permille>>>()?;
    model.memory = memory(json)?;
    model.temperature = temperature(json)?;
    let pressures = member(json, "pressure")?;
    model.pressure = PressureView {
        cpu: pressure(pressures, "cpu")?,
        memory: pressure(pressures, "memory")?,
        io: pressure(pressures, "io")?,
    };
    Ok(())
}

/// The tables: processes, units, interfaces, disks and mounts.
fn table_fields(json: &Json, model: &mut Model, history_len: usize) -> Field<()> {
    model.processes = items(json, "processes")?
        .iter()
        .map(process)
        .collect::<Field<Vec<ProcessRow>>>()?;
    model.services = services(json)?;
    model.network = items(json, "network")?
        .iter()
        .map(|interface| net_row(interface, history_len))
        .collect::<Field<Vec<NetRow>>>()?;
    model.disks = items(json, "disks")?
        .iter()
        .map(disk_row)
        .collect::<Field<Vec<DiskRow>>>()?;
    model.mounts = items(json, "mounts")?
        .iter()
        .map(mount_row)
        .collect::<Field<Vec<MountRow>>>()?;
    Ok(())
}

fn net_row(json: &Json, history_len: usize) -> Field<NetRow> {
    Ok(NetRow {
        name: text(json, "name")?.to_owned(),
        received: rate(json, "received_bytes_per_second")?,
        sent: rate(json, "sent_bytes_per_second")?,
        // Filled in while loading, in recording order.
        received_history: History::new(history_len),
        sent_history: History::new(history_len),
    })
}

fn disk_row(json: &Json) -> Field<DiskRow> {
    Ok(DiskRow {
        name: text(json, "name")?.to_owned(),
        read: rate(json, "read_bytes_per_second")?,
        written: rate(json, "written_bytes_per_second")?,
    })
}

fn mount_row(json: &Json) -> Field<MountRow> {
    Ok(MountRow {
        mount_point: text(json, "mount_point")?.to_owned(),
        fs_type: text(json, "fs_type")?.to_owned(),
        total: Bytes::new(unsigned(json, "total_bytes")?),
        used: Bytes::new(unsigned(json, "used_bytes")?),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use borbatop_core::{parse_loadavg, parse_meminfo, parse_pressure, parse_systemctl_units};

    use super::*;

    fn sample_model() -> Model {
        let stat = "42 (x) S 1 1 1 0 -1 0 0 0 0 0 0 0 0 0 20 5 3 0 9 1 1 0";
        let parsed = borbatop_core::parse_process_stat(stat).unwrap();
        let mut model = Model::empty(4);
        model.kernel_release = "7.2.1-arch1".to_owned();
        model.uptime = Some(Duration::from_secs(90));
        model.load = Some(parse_loadavg("1.25 0.50 0.05 1/100 1\n").unwrap());
        model.cores = vec![Permille::new(100), Permille::new(900)];
        model.cpu_history.push(500);
        model.memory = Some(
            parse_meminfo(
                "MemTotal: 1000 kB\nMemFree: 100 kB\nMemAvailable: 400 kB\nSwapTotal: 200 kB\nSwapFree: 150 kB\n",
            )
            .unwrap(),
        );
        model.temperature = Some(MilliCelsius::new(61_000));
        model.pressure.cpu = Some(
            parse_pressure(
                "some avg10=1.50 avg60=2.00 avg300=0.10 total=1\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=0\n",
            )
            .unwrap(),
        );
        model.pressure.io = None;
        model.processes = vec![ProcessRow {
            pid: parsed.pid(),
            start_time: parsed.id().start_time(),
            parent: parsed.parent(),
            user: "borba".to_owned(),
            state: ProcessState::Sleeping,
            cpu: Permille::new(250),
            memory: Bytes::new(4_096),
            nice: parsed.nice(),
            threads: parsed.threads(),
            unit: "app.scope".to_owned(),
            package: "pkg".to_owned(),
            command: "/usr/bin/tool --flag \"quoted\"".to_owned(),
        }];
        model.services = Ok(parse_systemctl_units(
            "a.service loaded failed failed The A service\nb.timer loaded active waiting B timer\n",
        ));
        let mut history = History::new(4);
        history.push(2_048);
        model.network = vec![NetRow {
            name: "eth0".to_owned(),
            received: Some(BytesPerSecond::new(2_048)),
            sent: None,
            received_history: history,
            sent_history: {
                let mut sent = History::new(4);
                sent.push(0);
                sent
            },
        }];
        model.disks = vec![DiskRow {
            name: "nvme0n1".to_owned(),
            read: Some(BytesPerSecond::new(1)),
            written: Some(BytesPerSecond::new(2)),
        }];
        model.mounts = vec![MountRow {
            mount_point: "/".to_owned(),
            fs_type: "ext4".to_owned(),
            total: Bytes::new(1_000),
            used: Bytes::new(400),
        }];
        model
    }

    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("borbatop-{name}-{}.jsonl", std::process::id()))
    }

    #[test]
    fn a_snapshot_with_commands_loads_back_as_the_same_model() {
        let original = sample_model();
        let line = model_json(&original, CommandLines::Include).render();
        let path = temp_file("roundtrip");
        fs::write(&path, format!("{line}\n")).unwrap();

        let loaded = load(&path, 4).unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0], original);
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn without_commands_the_program_name_is_shown_instead() {
        let line = model_json(&sample_model(), CommandLines::Omit).render();
        let path = temp_file("nocommands");
        fs::write(&path, format!("{line}\n")).unwrap();

        let loaded = load(&path, 4).unwrap();

        assert_eq!(loaded[0].processes[0].command, "tool");
        assert!(!format!("{:?}", loaded[0].processes[0]).contains("--flag"));
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn histories_grow_along_the_recording_and_are_bounded() {
        let line = model_json(&sample_model(), CommandLines::Omit).render();
        let path = temp_file("history");
        fs::write(
            &path,
            format!("{line}\n{line}\n{line}\n{line}\n{line}\n{line}\n"),
        )
        .unwrap();

        let loaded = load(&path, 4).unwrap();

        assert_eq!(loaded.len(), 6);
        assert_eq!(loaded[0].cpu_history.len(), 1);
        assert_eq!(loaded[2].cpu_history.len(), 3);
        assert_eq!(
            loaded[5].cpu_history.len(),
            4,
            "bounded by the history length"
        );
        assert_eq!(loaded[5].network[0].received_history.len(), 4);
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn a_cut_off_last_line_is_ignored_but_a_broken_middle_line_is_an_error() {
        let line = model_json(&sample_model(), CommandLines::Omit).render();
        let cut = temp_file("cut");
        fs::write(&cut, format!("{line}\n{}", &line[..line.len() / 2])).unwrap();
        assert_eq!(load(&cut, 4).unwrap().len(), 1);
        fs::remove_file(&cut).unwrap();

        let broken = temp_file("broken");
        fs::write(&broken, format!("{line}\nnot json\n{line}\n")).unwrap();
        assert!(matches!(
            load(&broken, 4),
            Err(RecordingError::Json { line: 2, .. })
        ));
        fs::remove_file(&broken).unwrap();
    }

    #[test]
    fn missing_fields_name_the_line_and_the_field() {
        let path = temp_file("field");
        fs::write(&path, "{\"kernel\":\"x\"}\n").unwrap();
        let error = load(&path, 4).unwrap_err();
        assert!(
            matches!(
                error,
                RecordingError::Field {
                    line: 1,
                    field: "cpu_permille"
                } | RecordingError::Field { line: 1, .. }
            ),
            "{error}"
        );
        fs::remove_file(&path).unwrap();

        let empty = temp_file("empty");
        fs::write(&empty, "\n\n").unwrap();
        assert!(matches!(load(&empty, 4), Err(RecordingError::Empty { .. })));
        fs::remove_file(&empty).unwrap();

        assert!(matches!(
            load(Path::new("/nonexistent/recording.jsonl"), 4),
            Err(RecordingError::Io { .. })
        ));
    }

    #[test]
    fn recording_refuses_to_overwrite_and_keeps_the_file_private() {
        let path = temp_file("private");
        let settings = Settings {
            interval: Duration::from_millis(200),
            theme: borbatop_tui::ThemeKind::Borba,
            tree: false,
            show_package: true,
            mouse: true,
            history_len: 8,
            alerts: borbatop_tui::AlertThresholds::default(),
        };

        record(settings, &path, 2, CommandLines::Omit).unwrap();

        let mode =
            std::os::unix::fs::PermissionsExt::mode(&fs::metadata(&path).unwrap().permissions());
        assert_eq!(mode & 0o777, 0o600);
        let models = load(&path, 8).unwrap();
        assert_eq!(models.len(), 2);
        assert_ne!(models[1].processes, []);

        let again = record(settings, &path, 1, CommandLines::Omit).unwrap_err();
        assert!(again.to_string().contains("exists"), "{again}");
        fs::remove_file(&path).unwrap();
    }
}
