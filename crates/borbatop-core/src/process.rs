//! Process identity and the parser of `/proc/[pid]/stat`.

use std::error::Error;
use std::fmt;
use std::str::FromStr;

use crate::cpu::CpuTicks;

const RUNNING_CODE: char = 'R';
const SLEEPING_CODE: char = 'S';
const DISK_SLEEP_CODE: char = 'D';
const ZOMBIE_CODE: char = 'Z';
const STOPPED_CODE: char = 'T';
const TRACING_STOP_CODE: char = 't';
const DEAD_CODE: char = 'X';
const IDLE_CODE: char = 'I';

// Positions of the fields we read, counted after the `)` that closes `comm`;
// field 3 (`state`) of `proc_pid_stat(5)` is index 0.
const STATE_INDEX: usize = 0;
const USER_TIME_INDEX: usize = 11;
const SYSTEM_TIME_INDEX: usize = 12;
const THREADS_INDEX: usize = 17;
const START_TIME_INDEX: usize = 19;
const RESIDENT_PAGES_INDEX: usize = 21;

/// Placeholder printed instead of the command name in `Debug` output (LGPD).
const REDACTED_COMM: &str = "<redacted>";

/// Process identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pid(u32);

impl Pid {
    /// Wraps a raw process identifier.
    ///
    /// # Arguments
    ///
    /// * `raw` - numeric PID as the kernel reports it
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw numeric PID.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for Pid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Moment a process started, in clock ticks after boot.
///
/// Together with the [`Pid`] it identifies a process for its whole life: the kernel may
/// hand the same PID to a different process later, but never with the same start time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StartTime(u64);

impl StartTime {
    /// The raw tick count since boot.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Identity of one process incarnation: PID plus start time.
///
/// Use it, not the bare [`Pid`], as the key that links two consecutive samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProcessId {
    pid: Pid,
    start_time: StartTime,
}

impl ProcessId {
    /// The process identifier.
    #[must_use]
    pub const fn pid(self) -> Pid {
        self.pid
    }

    /// When the process started.
    #[must_use]
    pub const fn start_time(self) -> StartTime {
        self.start_time
    }
}

/// Number of threads in a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ThreadCount(u32);

impl ThreadCount {
    /// The raw thread count.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for ThreadCount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// An amount of memory in pages; the page size is a property of the machine, not of the type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pages(u64);

impl Pages {
    /// The raw page count.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for Pages {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Process state, the third field of `/proc/[pid]/stat` (see `proc_pid_stat(5)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    /// Running or runnable.
    Running,
    /// Interruptible sleep.
    Sleeping,
    /// Uninterruptible sleep, usually waiting on I/O.
    DiskSleep,
    /// Terminated, not yet reaped by its parent.
    Zombie,
    /// Stopped by a job-control signal.
    Stopped,
    /// Stopped by a tracer.
    TracingStop,
    /// Dead; should never be observed.
    Dead,
    /// Idle kernel thread.
    Idle,
}

/// State code that no [`ProcessState`] recognizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownStateCode(char);

impl fmt::Display for UnknownStateCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown process state code: {:?}", self.0)
    }
}

impl Error for UnknownStateCode {}

impl ProcessState {
    /// Parses the state character read from `/proc/[pid]/stat`.
    ///
    /// # Arguments
    ///
    /// * `code` - raw state character
    ///
    /// # Errors
    ///
    /// Returns [`UnknownStateCode`] when `code` matches no state.
    pub const fn from_code(code: char) -> Result<Self, UnknownStateCode> {
        match code {
            RUNNING_CODE => Ok(Self::Running),
            SLEEPING_CODE => Ok(Self::Sleeping),
            DISK_SLEEP_CODE => Ok(Self::DiskSleep),
            ZOMBIE_CODE => Ok(Self::Zombie),
            STOPPED_CODE => Ok(Self::Stopped),
            TRACING_STOP_CODE => Ok(Self::TracingStop),
            DEAD_CODE => Ok(Self::Dead),
            IDLE_CODE => Ok(Self::Idle),
            unknown => Err(UnknownStateCode(unknown)),
        }
    }
}

/// A field of `/proc/[pid]/stat` that the parser reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatField {
    /// Process identifier.
    Pid,
    /// Single-character state.
    State,
    /// Time spent in user mode.
    UserTime,
    /// Time spent in kernel mode.
    SystemTime,
    /// Number of threads.
    Threads,
    /// Moment the process started, after boot.
    StartTime,
    /// Resident set size, in pages.
    ResidentPages,
}

impl fmt::Display for StatField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Pid => "pid",
            Self::State => "state",
            Self::UserTime => "utime",
            Self::SystemTime => "stime",
            Self::Threads => "num_threads",
            Self::StartTime => "starttime",
            Self::ResidentPages => "rss",
        };
        formatter.write_str(name)
    }
}

/// Why a `/proc/[pid]/stat` line could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatParseError {
    /// The parenthesized command name is absent.
    MissingComm,
    /// The line ends before this field.
    MissingField(StatField),
    /// This field is not a valid number.
    InvalidNumber(StatField),
    /// The state code is unknown.
    UnknownState(UnknownStateCode),
    /// User plus system time exceeds 64 bits.
    CpuTimeOverflow,
}

impl fmt::Display for StatParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingComm => write!(formatter, "stat line has no (comm)"),
            Self::MissingField(field) => write!(formatter, "stat line is missing {field}"),
            Self::InvalidNumber(field) => write!(formatter, "stat field {field} is not a number"),
            Self::UnknownState(error) => error.fmt(formatter),
            Self::CpuTimeOverflow => write!(formatter, "utime + stime overflowed"),
        }
    }
}

impl Error for StatParseError {}

/// One process as seen in a single read of `/proc/[pid]/stat`.
///
/// `Debug` hides the command name: it is user-controlled text that may carry personal
/// data, and must not reach logs by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct ProcessSample {
    id: ProcessId,
    comm: String,
    state: ProcessState,
    cpu_ticks: CpuTicks,
    threads: ThreadCount,
    resident_pages: Pages,
}

impl fmt::Debug for ProcessSample {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessSample")
            .field("id", &self.id)
            .field("comm", &REDACTED_COMM)
            .field("state", &self.state)
            .field("cpu_ticks", &self.cpu_ticks)
            .field("threads", &self.threads)
            .field("resident_pages", &self.resident_pages)
            .finish()
    }
}

impl ProcessSample {
    /// Identity of this process incarnation (PID plus start time).
    #[must_use]
    pub const fn id(&self) -> ProcessId {
        self.id
    }

    /// Process identifier.
    #[must_use]
    pub const fn pid(&self) -> Pid {
        self.id.pid
    }

    /// Executable name, truncated by the kernel to 15 bytes.
    ///
    /// Bytes that are not valid UTF-8 (a multibyte character cut by the truncation) are
    /// the caller's to decode; the parser only ever sees text.
    #[must_use]
    pub fn comm(&self) -> &str {
        &self.comm
    }

    /// Scheduler state.
    #[must_use]
    pub const fn state(&self) -> ProcessState {
        self.state
    }

    /// Cumulative user plus kernel CPU time.
    #[must_use]
    pub const fn cpu_ticks(&self) -> CpuTicks {
        self.cpu_ticks
    }

    /// Number of threads in the process.
    #[must_use]
    pub const fn threads(&self) -> ThreadCount {
        self.threads
    }

    /// Resident set size.
    ///
    /// The kernel may report a transiently negative value; it is clamped to zero.
    #[must_use]
    pub const fn resident_pages(&self) -> Pages {
        self.resident_pages
    }
}

/// Parses one `/proc/[pid]/stat` line.
///
/// The command name sits between the first `(` and the **last** `)`: it may itself contain
/// spaces and parentheses, so the line cannot be split on whitespace from the start.
///
/// # Arguments
///
/// * `line` - full contents of `/proc/[pid]/stat`, already decoded to text
///
/// # Errors
///
/// * [`StatParseError::MissingComm`] when `(` or `)` is absent.
/// * [`StatParseError::MissingField`] when the line ends before a required field.
/// * [`StatParseError::InvalidNumber`] when a numeric field is malformed.
/// * [`StatParseError::UnknownState`] when the state code is unknown.
/// * [`StatParseError::CpuTimeOverflow`] when utime + stime exceeds 64 bits.
///
/// # Examples
///
/// ```
/// use borbatop_core::{parse_process_stat, ProcessState};
///
/// let line = "1234 (my (odd) proc) S 1 1234 1234 0 -1 4194560 100 0 0 0 5 3 0 0 20 0 4 0 99 1000 250 0";
/// let sample = parse_process_stat(line).map_err(|error| error.to_string())?;
/// assert_eq!(sample.comm(), "my (odd) proc");
/// assert_eq!(sample.state(), ProcessState::Sleeping);
/// assert_eq!(sample.cpu_ticks().0, 8); // utime 5 + stime 3
/// assert_eq!(sample.threads().get(), 4);
/// assert_eq!(sample.id().start_time().get(), 99);
/// assert_eq!(sample.resident_pages().get(), 250);
/// # Ok::<(), String>(())
/// ```
pub fn parse_process_stat(line: &str) -> Result<ProcessSample, StatParseError> {
    let (head, tail) = line.split_once('(').ok_or(StatParseError::MissingComm)?;
    let (comm, rest) = tail.rsplit_once(')').ok_or(StatParseError::MissingComm)?;
    let pid = head
        .trim()
        .parse::<u32>()
        .map_err(|_| StatParseError::InvalidNumber(StatField::Pid))?;

    let mut state: Option<ProcessState> = None;
    let mut user_time: Option<u64> = None;
    let mut system_time: Option<u64> = None;
    let mut threads: Option<u32> = None;
    let mut start_time: Option<u64> = None;
    let mut resident_pages: Option<i64> = None;

    for (index, token) in rest.split_ascii_whitespace().enumerate() {
        match index {
            STATE_INDEX => state = Some(parse_state(token)?),
            USER_TIME_INDEX => user_time = Some(parse_number(token, StatField::UserTime)?),
            SYSTEM_TIME_INDEX => system_time = Some(parse_number(token, StatField::SystemTime)?),
            THREADS_INDEX => threads = Some(parse_number(token, StatField::Threads)?),
            START_TIME_INDEX => start_time = Some(parse_number(token, StatField::StartTime)?),
            RESIDENT_PAGES_INDEX => {
                resident_pages = Some(parse_number(token, StatField::ResidentPages)?);
            }
            _ => {}
        }
        if index >= RESIDENT_PAGES_INDEX {
            break;
        }
    }

    let user_time = user_time.ok_or(StatParseError::MissingField(StatField::UserTime))?;
    let system_time = system_time.ok_or(StatParseError::MissingField(StatField::SystemTime))?;
    let cpu_ticks = user_time
        .checked_add(system_time)
        .ok_or(StatParseError::CpuTimeOverflow)?;
    let start_time = start_time.ok_or(StatParseError::MissingField(StatField::StartTime))?;
    let resident = resident_pages.ok_or(StatParseError::MissingField(StatField::ResidentPages))?;

    Ok(ProcessSample {
        id: ProcessId {
            pid: Pid(pid),
            start_time: StartTime(start_time),
        },
        comm: comm.to_owned(),
        state: state.ok_or(StatParseError::MissingField(StatField::State))?,
        cpu_ticks: CpuTicks(cpu_ticks),
        threads: ThreadCount(threads.ok_or(StatParseError::MissingField(StatField::Threads))?),
        resident_pages: Pages(resident.max(0).unsigned_abs()),
    })
}

fn parse_state(token: &str) -> Result<ProcessState, StatParseError> {
    let code = token
        .chars()
        .next()
        .ok_or(StatParseError::MissingField(StatField::State))?;
    ProcessState::from_code(code).map_err(StatParseError::UnknownState)
}

fn parse_number<T: FromStr>(token: &str, field: StatField) -> Result<T, StatParseError> {
    token
        .parse()
        .map_err(|_| StatParseError::InvalidNumber(field))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const LINE: &str =
        "10 (init) S 0 1 1 0 -1 4194560 0 0 0 0 7 3 0 0 20 0 2 0 1 100 50 18446744073709551615";

    #[test]
    fn parses_every_known_code() {
        assert_eq!(ProcessState::from_code('R'), Ok(ProcessState::Running));
        assert_eq!(ProcessState::from_code('I'), Ok(ProcessState::Idle));
        assert_eq!(ProcessState::from_code('t'), Ok(ProcessState::TracingStop));
    }

    #[test]
    fn rejects_unknown_code() {
        assert_eq!(ProcessState::from_code('?'), Err(UnknownStateCode('?')));
    }

    #[test]
    fn parses_a_regular_line() {
        let sample = parse_process_stat(LINE).unwrap();
        assert_eq!(sample.pid(), Pid::new(10));
        assert_eq!(sample.id().start_time(), StartTime(1));
        assert_eq!(sample.comm(), "init");
        assert_eq!(sample.state(), ProcessState::Sleeping);
        assert_eq!(sample.cpu_ticks(), CpuTicks(10));
        assert_eq!(sample.threads(), ThreadCount(2));
        assert_eq!(sample.resident_pages(), Pages(50));
    }

    #[test]
    fn reused_pid_has_a_different_identity() {
        let reborn = LINE.replace(" 0 1 100 50 ", " 0 9 100 50 ");
        let first = parse_process_stat(LINE).unwrap();
        let second = parse_process_stat(&reborn).unwrap();
        assert_eq!(first.pid(), second.pid());
        assert_ne!(first.id(), second.id());
    }

    #[test]
    fn comm_may_contain_spaces_and_parentheses() {
        let line = LINE.replace("(init)", "(a) b (c)");
        let sample = parse_process_stat(&line).unwrap();
        assert_eq!(sample.comm(), "a) b (c");
        assert_eq!(sample.cpu_ticks(), CpuTicks(10));
    }

    #[test]
    fn negative_rss_is_clamped_to_zero() {
        let line = LINE.replace(" 100 50 ", " 100 -3 ");
        let sample = parse_process_stat(&line).unwrap();
        assert_eq!(sample.resident_pages(), Pages(0));
    }

    #[test]
    fn debug_output_hides_the_command_name() {
        let line = LINE.replace("(init)", "(secret-name)");
        let sample = parse_process_stat(&line).unwrap();
        let printed = format!("{sample:?}");
        assert!(!printed.contains("secret-name"));
        assert!(printed.contains(REDACTED_COMM));
    }

    #[test]
    fn rejects_missing_comm() {
        assert_eq!(
            parse_process_stat("10 init S 0"),
            Err(StatParseError::MissingComm)
        );
        assert_eq!(
            parse_process_stat("10 (init S 0"),
            Err(StatParseError::MissingComm)
        );
    }

    #[test]
    fn rejects_bad_pid() {
        assert_eq!(
            parse_process_stat("x (init) S"),
            Err(StatParseError::InvalidNumber(StatField::Pid))
        );
    }

    #[test]
    fn rejects_truncated_line() {
        assert_eq!(
            parse_process_stat("10 (init) S 0 1 1"),
            Err(StatParseError::MissingField(StatField::UserTime))
        );
    }

    #[test]
    fn rejects_unknown_state_and_garbled_number() {
        let unknown = LINE.replace(") S ", ") Q ");
        assert_eq!(
            parse_process_stat(&unknown),
            Err(StatParseError::UnknownState(UnknownStateCode('Q')))
        );
        let garbled = LINE.replace(" 7 3 ", " x 3 ");
        assert_eq!(
            parse_process_stat(&garbled),
            Err(StatParseError::InvalidNumber(StatField::UserTime))
        );
    }

    #[test]
    fn cpu_time_overflow_is_an_error() {
        let max = u64::MAX.to_string();
        let line = LINE.replace(" 7 3 ", &format!(" {max} 3 "));
        assert_eq!(
            parse_process_stat(&line),
            Err(StatParseError::CpuTimeOverflow)
        );
    }
}
