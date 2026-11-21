//! CPU accounting: cumulative counters, deltas and usage.

use std::error::Error;
use std::fmt;
use std::num::NonZeroU64;

const PERMILLE_SCALE: u64 = 1_000;
const PERMILLE_PER_PERCENT: u64 = 10;
const AGGREGATE_LABEL: &str = "cpu";
const MIN_CPU_FIELDS: usize = 4;
const CPU_FIELD_COUNT: usize = 8;

/// Cumulative CPU time (utime + stime, in clock ticks) of one process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuTicks(pub u64);

/// Wall-clock time between two samples, in clock ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElapsedTicks(pub u64);

/// CPU usage in permille of one core: 1000 is one fully busy core, 2000 is two.
///
/// Displays as a percentage with one decimal digit.
///
/// # Examples
///
/// ```
/// use borbatop_core::Permille;
///
/// assert_eq!(Permille::new(3_519).to_string(), "351.9");
/// assert_eq!(format!("{:>7}", Permille::new(49)), "    4.9");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Permille(u64);

impl Permille {
    /// Wraps a raw permille value.
    ///
    /// # Arguments
    ///
    /// * `raw` - usage in permille of one core
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw permille value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for Permille {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let whole = self.0 / PERMILLE_PER_PERCENT;
        let tenth = self.0 % PERMILLE_PER_PERCENT;
        formatter.pad(&format!("{whole}.{tenth}"))
    }
}

/// Two consecutive readings of the same counter.
///
/// Carrying both in one value, with named accessors, keeps `previous` and `current`
/// from being swapped at a call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SamplePair<T> {
    previous: T,
    current: T,
}

impl<T: Copy> SamplePair<T> {
    /// Pairs two readings.
    ///
    /// # Arguments
    ///
    /// * `previous` - reading taken first
    /// * `current` - reading taken last
    #[must_use]
    pub const fn new(previous: T, current: T) -> Self {
        Self { previous, current }
    }

    /// The reading taken first.
    #[must_use]
    pub const fn previous(&self) -> T {
        self.previous
    }

    /// The reading taken last.
    #[must_use]
    pub const fn current(&self) -> T {
        self.current
    }
}

/// Why a CPU usage delta could not be computed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleError {
    /// Both samples share the same instant.
    NoTimeElapsed,
    /// The counter went backwards: the PID was most likely reused.
    CounterWentBackwards,
    /// The result does not fit in 64 bits.
    Overflow,
    /// The machine reports zero cores.
    NoCores,
}

impl fmt::Display for SampleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoTimeElapsed => write!(formatter, "no time elapsed between samples"),
            Self::CounterWentBackwards => write!(formatter, "cpu counter went backwards"),
            Self::Overflow => write!(formatter, "cpu usage overflowed"),
            Self::NoCores => write!(formatter, "machine reports zero cores"),
        }
    }
}

impl Error for SampleError {}

/// CPU usage of one process over the sampling window.
///
/// # Arguments
///
/// * `window` - cumulative ticks at the previous and at the current sample
/// * `elapsed` - wall-clock ticks between both samples, as seen by one core
///
/// # Errors
///
/// * [`SampleError::NoTimeElapsed`] when `elapsed` is zero.
/// * [`SampleError::CounterWentBackwards`] when the counter decreased.
/// * [`SampleError::Overflow`] when the scaled delta exceeds `u64::MAX`.
///
/// # Examples
///
/// ```
/// use borbatop_core::{cpu_permille, CpuTicks, ElapsedTicks, Permille, SamplePair};
///
/// let window = SamplePair::new(CpuTicks(100), CpuTicks(150));
/// let usage = cpu_permille(window, ElapsedTicks(100));
/// assert_eq!(usage, Ok(Permille::new(500))); // 50 ticks of CPU in 100 ticks of wall time
/// ```
pub fn cpu_permille(
    window: SamplePair<CpuTicks>,
    elapsed: ElapsedTicks,
) -> Result<Permille, SampleError> {
    let ElapsedTicks(elapsed_ticks) = elapsed;
    if elapsed_ticks == 0 {
        return Err(SampleError::NoTimeElapsed);
    }

    let CpuTicks(previous) = window.previous();
    let CpuTicks(current) = window.current();
    let delta = current
        .checked_sub(previous)
        .ok_or(SampleError::CounterWentBackwards)?;

    delta
        .checked_mul(PERMILLE_SCALE)
        .and_then(|scaled| scaled.checked_div(elapsed_ticks))
        .map(Permille)
        .ok_or(SampleError::Overflow)
}

/// Cumulative busy and idle ticks of the whole machine or of one core.
///
/// `busy` is user + nice + system + irq + softirq + steal; `idle` is idle + iowait.
/// Guest time is already counted inside user time by the kernel, so it is not added again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuTimes {
    busy: u64,
    idle: u64,
}

impl CpuTimes {
    /// Builds a pair of cumulative counters.
    ///
    /// # Arguments
    ///
    /// * `busy` - cumulative ticks spent doing work
    /// * `idle` - cumulative ticks spent idle or waiting on I/O
    #[must_use]
    pub const fn new(busy: u64, idle: u64) -> Self {
        Self { busy, idle }
    }

    /// Cumulative ticks spent doing work.
    #[must_use]
    pub const fn busy(self) -> u64 {
        self.busy
    }

    /// Cumulative ticks spent idle or waiting on I/O.
    #[must_use]
    pub const fn idle(self) -> u64 {
        self.idle
    }

    /// Busy plus idle ticks.
    ///
    /// # Errors
    ///
    /// Returns [`SampleError::Overflow`] when the sum exceeds `u64::MAX`.
    pub fn all_ticks(self) -> Result<u64, SampleError> {
        self.busy
            .checked_add(self.idle)
            .ok_or(SampleError::Overflow)
    }
}

/// Machine-wide usage over the sampling window, in permille (1000 = fully busy).
///
/// # Arguments
///
/// * `window` - machine-wide counters at the previous and at the current sample
///
/// # Errors
///
/// * [`SampleError::CounterWentBackwards`] when a counter decreased (CPU hotplug).
/// * [`SampleError::NoTimeElapsed`] when no tick passed between samples.
/// * [`SampleError::Overflow`] when the scaled delta exceeds `u64::MAX`.
///
/// # Examples
///
/// ```
/// use borbatop_core::{busy_permille, CpuTimes, Permille, SamplePair};
///
/// let window = SamplePair::new(CpuTimes::new(100, 900), CpuTimes::new(150, 950));
/// assert_eq!(busy_permille(window), Ok(Permille::new(500))); // 50 busy ticks out of 100
/// ```
pub fn busy_permille(window: SamplePair<CpuTimes>) -> Result<Permille, SampleError> {
    let previous = window.previous();
    let current = window.current();
    let busy_delta = current
        .busy
        .checked_sub(previous.busy)
        .ok_or(SampleError::CounterWentBackwards)?;
    let idle_delta = current
        .idle
        .checked_sub(previous.idle)
        .ok_or(SampleError::CounterWentBackwards)?;
    let elapsed = busy_delta
        .checked_add(idle_delta)
        .ok_or(SampleError::Overflow)?;
    if elapsed == 0 {
        return Err(SampleError::NoTimeElapsed);
    }

    busy_delta
        .checked_mul(PERMILLE_SCALE)
        .and_then(|scaled| scaled.checked_div(elapsed))
        .map(Permille)
        .ok_or(SampleError::Overflow)
}

/// Wall-clock ticks elapsed between two machine-wide samples, as seen by a single core.
///
/// Dividing the machine-wide delta by the core count is what makes a per-process
/// [`cpu_permille`] read 1000 for one fully busy core and 2000 for two.
///
/// # Arguments
///
/// * `window` - machine-wide counters at the previous and at the current sample
/// * `cores` - number of cores accounted in the counters
///
/// # Errors
///
/// * [`SampleError::NoCores`] when `cores` is zero.
/// * [`SampleError::CounterWentBackwards`] when the total decreased.
/// * [`SampleError::Overflow`] when a sum or the core count exceeds 64 bits.
pub fn elapsed_per_core(
    window: SamplePair<CpuTimes>,
    cores: usize,
) -> Result<ElapsedTicks, SampleError> {
    let core_count = u64::try_from(cores).map_err(|_| SampleError::Overflow)?;
    let core_count = NonZeroU64::new(core_count).ok_or(SampleError::NoCores)?;

    let delta = window
        .current()
        .all_ticks()?
        .checked_sub(window.previous().all_ticks()?)
        .ok_or(SampleError::CounterWentBackwards)?;

    Ok(ElapsedTicks(delta / core_count))
}

/// Why `/proc/stat` could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuStatError {
    /// The first line is not the aggregate `cpu` line.
    MissingAggregate,
    /// A cpu line has fewer than the four mandatory counters.
    MissingField,
    /// A counter is not an unsigned integer.
    InvalidNumber,
    /// A sum of counters exceeds 64 bits.
    Overflow,
}

impl fmt::Display for CpuStatError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingAggregate => write!(formatter, "/proc/stat has no aggregate cpu line"),
            Self::MissingField => write!(formatter, "cpu line has too few counters"),
            Self::InvalidNumber => write!(formatter, "cpu counter is not a number"),
            Self::Overflow => write!(formatter, "cpu counters overflowed"),
        }
    }
}

impl Error for CpuStatError {}

/// Machine-wide and per-core counters read from `/proc/stat`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuSnapshot {
    total: CpuTimes,
    cores: Vec<CpuTimes>,
}

impl CpuSnapshot {
    /// Counters of the whole machine.
    #[must_use]
    pub const fn total(&self) -> CpuTimes {
        self.total
    }

    /// Counters of each core, in kernel order.
    #[must_use]
    pub fn cores(&self) -> &[CpuTimes] {
        &self.cores
    }
}

/// Parses the leading `cpu` lines of `/proc/stat` (see `proc_stat(5)`).
///
/// # Arguments
///
/// * `text` - full contents of `/proc/stat`
///
/// # Errors
///
/// * [`CpuStatError::MissingAggregate`] when the first line is not `cpu ...`.
/// * [`CpuStatError::MissingField`] when a cpu line has fewer than four counters.
/// * [`CpuStatError::InvalidNumber`] when a counter is not an unsigned integer.
/// * [`CpuStatError::Overflow`] when summing counters exceeds 64 bits.
///
/// # Examples
///
/// ```
/// use borbatop_core::parse_cpu_stat;
///
/// let text = "cpu  10 0 5 80 5 0 0 0 0 0\ncpu0 10 0 5 80 5 0 0 0 0 0\nintr 1\n";
/// let snapshot = parse_cpu_stat(text).map_err(|error| error.to_string())?;
/// assert_eq!(snapshot.total().busy(), 15);
/// assert_eq!(snapshot.cores().len(), 1);
/// # Ok::<(), String>(())
/// ```
pub fn parse_cpu_stat(text: &str) -> Result<CpuSnapshot, CpuStatError> {
    let mut lines = text.lines();
    let aggregate = lines
        .next()
        .and_then(|line| line.strip_prefix(AGGREGATE_LABEL))
        .filter(|rest| rest.starts_with(char::is_whitespace))
        .ok_or(CpuStatError::MissingAggregate)?;
    let total = parse_cpu_fields(aggregate)?;

    let core_lines = text
        .lines()
        .filter(|line| line.starts_with(AGGREGATE_LABEL))
        .count()
        .saturating_sub(1);
    let mut cores = Vec::with_capacity(core_lines);

    for line in lines {
        let Some(rest) = line.strip_prefix(AGGREGATE_LABEL) else {
            break;
        };
        let Some((index, fields)) = rest.split_once(char::is_whitespace) else {
            break;
        };
        if index.is_empty() || !index.bytes().all(|byte| byte.is_ascii_digit()) {
            break;
        }
        cores.push(parse_cpu_fields(fields)?);
    }

    Ok(CpuSnapshot { total, cores })
}

fn parse_cpu_fields(fields: &str) -> Result<CpuTimes, CpuStatError> {
    let mut counters = [0_u64; CPU_FIELD_COUNT];
    let mut seen = 0_usize;

    for (slot, token) in counters.iter_mut().zip(fields.split_ascii_whitespace()) {
        *slot = token.parse().map_err(|_| CpuStatError::InvalidNumber)?;
        seen = seen.saturating_add(1);
    }
    if seen < MIN_CPU_FIELDS {
        return Err(CpuStatError::MissingField);
    }

    let [user, nice, system, idle, iowait, irq, softirq, steal] = counters;
    let busy = checked_sum(&[user, nice, system, irq, softirq, steal])?;
    let idle = checked_sum(&[idle, iowait])?;

    Ok(CpuTimes { busy, idle })
}

fn checked_sum(values: &[u64]) -> Result<u64, CpuStatError> {
    values
        .iter()
        .try_fold(0_u64, |sum, &value| sum.checked_add(value))
        .ok_or(CpuStatError::Overflow)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const FIXTURE: &str = "cpu  100 5 50 800 20 3 2 1 0 0\n\
                           cpu0 50 2 25 400 10 1 1 0 0 0\n\
                           cpu1 50 3 25 400 10 2 1 1 0 0\n\
                           intr 12345 1 2\nctxt 999\n";

    fn ticks(previous: u64, current: u64) -> SamplePair<CpuTicks> {
        SamplePair::new(CpuTicks(previous), CpuTicks(current))
    }

    fn times(previous: (u64, u64), current: (u64, u64)) -> SamplePair<CpuTimes> {
        SamplePair::new(
            CpuTimes::new(previous.0, previous.1),
            CpuTimes::new(current.0, current.1),
        )
    }

    #[test]
    fn detects_pid_reuse() {
        let usage = cpu_permille(ticks(500, 10), ElapsedTicks(100));
        assert_eq!(usage, Err(SampleError::CounterWentBackwards));
    }

    #[test]
    fn rejects_zero_window() {
        let usage = cpu_permille(ticks(1, 2), ElapsedTicks(0));
        assert_eq!(usage, Err(SampleError::NoTimeElapsed));
    }

    #[test]
    fn multi_threaded_process_exceeds_one_core() {
        let usage = cpu_permille(ticks(0, 300), ElapsedTicks(100));
        assert_eq!(usage, Ok(Permille::new(3_000)));
    }

    #[test]
    fn overflow_is_an_error() {
        let usage = cpu_permille(ticks(0, u64::MAX), ElapsedTicks(1));
        assert_eq!(usage, Err(SampleError::Overflow));
    }

    #[test]
    fn permille_displays_one_decimal_and_honors_width() {
        assert_eq!(Permille::new(0).to_string(), "0.0");
        assert_eq!(Permille::new(1_000).to_string(), "100.0");
        assert_eq!(Permille::new(3_519).to_string(), "351.9");
        assert_eq!(format!("{:>7}", Permille::new(49)), "    4.9");
    }

    #[test]
    fn parses_aggregate_and_cores() {
        let snapshot = parse_cpu_stat(FIXTURE).unwrap();
        assert_eq!(snapshot.total(), CpuTimes::new(161, 820));
        assert_eq!(
            snapshot.cores(),
            &[CpuTimes::new(79, 410), CpuTimes::new(82, 410)]
        );
    }

    #[test]
    fn old_kernel_with_four_counters_is_accepted() {
        let snapshot = parse_cpu_stat("cpu  1 2 3 4\n").unwrap();
        assert_eq!(snapshot.total(), CpuTimes::new(6, 4));
        assert_eq!(snapshot.cores(), []);
    }

    #[test]
    fn core_scan_stops_at_the_first_non_core_line() {
        let text = "cpu  1 2 3 4\ncpu0 1 2 3 4\ncpufoo 9 9 9 9\ncpu1 1 2 3 4\n";
        let snapshot = parse_cpu_stat(text).unwrap();
        assert_eq!(snapshot.cores(), &[CpuTimes::new(6, 4)]);
    }

    #[test]
    fn rejects_missing_aggregate() {
        assert_eq!(
            parse_cpu_stat("intr 1\n"),
            Err(CpuStatError::MissingAggregate)
        );
        assert_eq!(parse_cpu_stat(""), Err(CpuStatError::MissingAggregate));
        assert_eq!(
            parse_cpu_stat("cpu0 1 2 3 4\n"),
            Err(CpuStatError::MissingAggregate)
        );
    }

    #[test]
    fn rejects_short_and_garbled_lines() {
        assert_eq!(
            parse_cpu_stat("cpu  1 2 3\n"),
            Err(CpuStatError::MissingField)
        );
        assert_eq!(
            parse_cpu_stat("cpu  1 2 x 4\n"),
            Err(CpuStatError::InvalidNumber)
        );
    }

    #[test]
    fn machine_usage_is_busy_share_of_window() {
        let usage = busy_permille(times((100, 900), (300, 1_700)));
        assert_eq!(usage, Ok(Permille::new(200)));
    }

    #[test]
    fn machine_usage_rejects_idle_window_and_hotplug() {
        assert_eq!(
            busy_permille(times((5, 5), (5, 5))),
            Err(SampleError::NoTimeElapsed)
        );
        assert_eq!(
            busy_permille(times((9, 9), (1, 1))),
            Err(SampleError::CounterWentBackwards)
        );
    }

    #[test]
    fn elapsed_is_machine_delta_per_core() {
        let elapsed = elapsed_per_core(times((0, 0), (300, 500)), 4);
        assert_eq!(elapsed, Ok(ElapsedTicks(200)));
        let none = elapsed_per_core(times((0, 0), (1, 1)), 0);
        assert_eq!(none, Err(SampleError::NoCores));
    }

    #[test]
    fn elapsed_rejects_backwards_total() {
        let elapsed = elapsed_per_core(times((10, 10), (1, 1)), 2);
        assert_eq!(elapsed, Err(SampleError::CounterWentBackwards));
    }
}
