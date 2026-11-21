//! Parsers for `/proc/meminfo`, `/proc/loadavg`, `/proc/uptime`, PSI and hwmon temperatures.

use std::error::Error;
use std::fmt;
use std::time::Duration;

use crate::bytes::Bytes;
use crate::decimal::Hundredths;

const TOTAL_KEY: &str = "MemTotal";
const AVAILABLE_KEY: &str = "MemAvailable";
const SWAP_TOTAL_KEY: &str = "SwapTotal";
const SWAP_FREE_KEY: &str = "SwapFree";
const MILLIDEGREES_PER_DEGREE: i32 = 1_000;
const HALF_DEGREE: i32 = 500;

/// A `/proc/meminfo` field that the parser requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemField {
    /// `MemTotal`.
    Total,
    /// `MemAvailable`.
    Available,
    /// `SwapTotal`.
    SwapTotal,
    /// `SwapFree`.
    SwapFree,
}

impl fmt::Display for MemField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Total => TOTAL_KEY,
            Self::Available => AVAILABLE_KEY,
            Self::SwapTotal => SWAP_TOTAL_KEY,
            Self::SwapFree => SWAP_FREE_KEY,
        };
        formatter.write_str(name)
    }
}

/// Why `/proc/meminfo` could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemInfoError {
    /// A required field is absent.
    Missing(MemField),
    /// A required field is not an unsigned integer, or overflows when converted to bytes.
    Invalid(MemField),
}

impl fmt::Display for MemInfoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(field) => write!(formatter, "/proc/meminfo is missing {field}"),
            Self::Invalid(field) => write!(formatter, "/proc/meminfo has an invalid {field}"),
        }
    }
}

impl Error for MemInfoError {}

/// Memory and swap, as reported by `/proc/meminfo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemInfo {
    total: Bytes,
    available: Bytes,
    swap_total: Bytes,
    swap_free: Bytes,
}

impl MemInfo {
    /// Memory and swap from already-known amounts (for example, a recorded sample).
    ///
    /// # Arguments
    ///
    /// * `total` - physical memory installed
    /// * `available` - memory that can be given to programs without swapping
    /// * `swap_total` - swap space configured
    /// * `swap_free` - swap space not in use
    #[must_use]
    pub const fn new(total: Bytes, available: Bytes, swap_total: Bytes, swap_free: Bytes) -> Self {
        Self {
            total,
            available,
            swap_total,
            swap_free,
        }
    }

    /// Physical memory installed.
    #[must_use]
    pub const fn total(&self) -> Bytes {
        self.total
    }

    /// Memory that can be given to programs without swapping.
    #[must_use]
    pub const fn available(&self) -> Bytes {
        self.available
    }

    /// Memory in use: total minus available.
    #[must_use]
    pub const fn used(&self) -> Bytes {
        self.total.saturating_sub(self.available)
    }

    /// Swap space configured.
    #[must_use]
    pub const fn swap_total(&self) -> Bytes {
        self.swap_total
    }

    /// Swap space in use.
    #[must_use]
    pub const fn swap_used(&self) -> Bytes {
        self.swap_total.saturating_sub(self.swap_free)
    }
}

/// Parses `/proc/meminfo`.
///
/// # Arguments
///
/// * `text` - full contents of `/proc/meminfo`
///
/// # Errors
///
/// * [`MemInfoError::Missing`] when `MemTotal`, `MemAvailable`, `SwapTotal` or `SwapFree` is absent.
/// * [`MemInfoError::Invalid`] when one of them is not a number of kibibytes.
///
/// # Examples
///
/// ```
/// use borbatop_core::parse_meminfo;
///
/// let text = "MemTotal: 1024 kB\nMemAvailable: 512 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n";
/// let info = parse_meminfo(text).map_err(|error| error.to_string())?;
/// assert_eq!(info.used().get(), 512 * 1024);
/// # Ok::<(), String>(())
/// ```
pub fn parse_meminfo(text: &str) -> Result<MemInfo, MemInfoError> {
    let mut total = None;
    let mut available = None;
    let mut swap_total = None;
    let mut swap_free = None;

    for line in text.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let slot = match key {
            TOTAL_KEY => (&mut total, MemField::Total),
            AVAILABLE_KEY => (&mut available, MemField::Available),
            SWAP_TOTAL_KEY => (&mut swap_total, MemField::SwapTotal),
            SWAP_FREE_KEY => (&mut swap_free, MemField::SwapFree),
            _ => continue,
        };
        *slot.0 = Some(parse_kib(rest, slot.1)?);
    }

    Ok(MemInfo {
        total: total.ok_or(MemInfoError::Missing(MemField::Total))?,
        available: available.ok_or(MemInfoError::Missing(MemField::Available))?,
        swap_total: swap_total.ok_or(MemInfoError::Missing(MemField::SwapTotal))?,
        swap_free: swap_free.ok_or(MemInfoError::Missing(MemField::SwapFree))?,
    })
}

fn parse_kib(rest: &str, field: MemField) -> Result<Bytes, MemInfoError> {
    rest.split_ascii_whitespace()
        .next()
        .and_then(|token| token.parse::<u64>().ok())
        .and_then(Bytes::from_kib)
        .ok_or(MemInfoError::Invalid(field))
}

/// Why a small procfs file could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcTextError {
    /// The file does not have the expected shape.
    Malformed,
}

impl fmt::Display for ProcTextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed => write!(formatter, "procfs file is malformed"),
        }
    }
}

impl Error for ProcTextError {}

/// The three load averages of `/proc/loadavg`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadAvg {
    one: Hundredths,
    five: Hundredths,
    fifteen: Hundredths,
}

impl LoadAvg {
    /// Load averages from known values.
    ///
    /// # Arguments
    ///
    /// * `one` - average over the last minute
    /// * `five` - average over the last five minutes
    /// * `fifteen` - average over the last fifteen minutes
    #[must_use]
    pub const fn new(one: Hundredths, five: Hundredths, fifteen: Hundredths) -> Self {
        Self { one, five, fifteen }
    }

    /// Average over the last minute.
    #[must_use]
    pub const fn one(&self) -> Hundredths {
        self.one
    }

    /// Average over the last five minutes.
    #[must_use]
    pub const fn five(&self) -> Hundredths {
        self.five
    }

    /// Average over the last fifteen minutes.
    #[must_use]
    pub const fn fifteen(&self) -> Hundredths {
        self.fifteen
    }
}

/// Parses `/proc/loadavg`.
///
/// # Arguments
///
/// * `text` - full contents of `/proc/loadavg`
///
/// # Errors
///
/// Returns [`ProcTextError::Malformed`] when the first three tokens are not decimals.
///
/// # Examples
///
/// ```
/// use borbatop_core::parse_loadavg;
///
/// let load = parse_loadavg("3.91 5.13 4.87 3/1488 50250\n").map_err(|error| error.to_string())?;
/// assert_eq!(load.five().to_string(), "5.13");
/// # Ok::<(), String>(())
/// ```
pub fn parse_loadavg(text: &str) -> Result<LoadAvg, ProcTextError> {
    let mut tokens = text.split_ascii_whitespace().map(Hundredths::parse);
    match (tokens.next(), tokens.next(), tokens.next()) {
        (Some(Some(one)), Some(Some(five)), Some(Some(fifteen))) => {
            Ok(LoadAvg { one, five, fifteen })
        }
        _ => Err(ProcTextError::Malformed),
    }
}

/// Time since boot, from `/proc/uptime`.
///
/// # Arguments
///
/// * `text` - full contents of `/proc/uptime`
///
/// # Errors
///
/// Returns [`ProcTextError::Malformed`] when the first token is not a decimal number of seconds.
///
/// # Examples
///
/// ```
/// use borbatop_core::parse_uptime;
///
/// let uptime = parse_uptime("36365.04 326911.62\n").map_err(|error| error.to_string())?;
/// assert_eq!(uptime.as_secs(), 36_365);
/// # Ok::<(), String>(())
/// ```
pub fn parse_uptime(text: &str) -> Result<Duration, ProcTextError> {
    let token = text
        .split_ascii_whitespace()
        .next()
        .ok_or(ProcTextError::Malformed)?;
    let whole = token.split_once('.').map_or(token, |(whole, _)| whole);
    whole
        .parse::<u64>()
        .map(Duration::from_secs)
        .map_err(|_| ProcTextError::Malformed)
}

/// Averages of one `/proc/pressure/*` line over 10, 60 and 300 seconds, in percent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PressureLine {
    avg10: Hundredths,
    avg60: Hundredths,
    avg300: Hundredths,
}

impl PressureLine {
    /// A pressure line from known averages.
    ///
    /// # Arguments
    ///
    /// * `avg10` - share of the last 10 seconds spent stalled, in percent
    /// * `avg60` - share of the last 60 seconds
    /// * `avg300` - share of the last 300 seconds
    #[must_use]
    pub const fn new(avg10: Hundredths, avg60: Hundredths, avg300: Hundredths) -> Self {
        Self {
            avg10,
            avg60,
            avg300,
        }
    }

    /// Share of the last 10 seconds spent stalled, in percent.
    #[must_use]
    pub const fn avg10(&self) -> Hundredths {
        self.avg10
    }

    /// Share of the last 60 seconds spent stalled, in percent.
    #[must_use]
    pub const fn avg60(&self) -> Hundredths {
        self.avg60
    }

    /// Share of the last 300 seconds spent stalled, in percent.
    #[must_use]
    pub const fn avg300(&self) -> Hundredths {
        self.avg300
    }
}

/// Pressure-stall information of one resource (CPU, memory or I/O).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pressure {
    some: PressureLine,
    full: Option<PressureLine>,
}

impl Pressure {
    /// Pressure of one resource from known lines.
    ///
    /// # Arguments
    ///
    /// * `some` - time when at least one task was stalled
    /// * `full` - time when every task was stalled, when the kernel reports it
    #[must_use]
    pub const fn new(some: PressureLine, full: Option<PressureLine>) -> Self {
        Self { some, full }
    }

    /// Time when at least one task was stalled.
    #[must_use]
    pub const fn some(&self) -> PressureLine {
        self.some
    }

    /// Time when every task was stalled; absent on older kernels for CPU.
    #[must_use]
    pub const fn full(&self) -> Option<PressureLine> {
        self.full
    }
}

/// Parses one of `/proc/pressure/{cpu,memory,io}`.
///
/// # Arguments
///
/// * `text` - full contents of the pressure file
///
/// # Errors
///
/// Returns [`ProcTextError::Malformed`] when the `some` line is absent or lacks an average.
///
/// # Examples
///
/// ```
/// use borbatop_core::parse_pressure;
///
/// let text = "some avg10=0.00 avg60=0.14 avg300=0.40 total=9\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=0\n";
/// let pressure = parse_pressure(text).map_err(|error| error.to_string())?;
/// assert_eq!(pressure.some().avg60().to_string(), "0.14");
/// assert!(pressure.full().is_some());
/// # Ok::<(), String>(())
/// ```
pub fn parse_pressure(text: &str) -> Result<Pressure, ProcTextError> {
    let mut some = None;
    let mut full = None;

    for line in text.lines() {
        let mut tokens = line.split_ascii_whitespace();
        match tokens.next() {
            Some("some") => some = Some(parse_pressure_line(tokens)?),
            Some("full") => full = Some(parse_pressure_line(tokens)?),
            Some(_) | None => {}
        }
    }

    Ok(Pressure {
        some: some.ok_or(ProcTextError::Malformed)?,
        full,
    })
}

fn parse_pressure_line<'a>(
    tokens: impl Iterator<Item = &'a str>,
) -> Result<PressureLine, ProcTextError> {
    let mut avg10 = None;
    let mut avg60 = None;
    let mut avg300 = None;

    for token in tokens {
        let Some((key, value)) = token.split_once('=') else {
            continue;
        };
        match key {
            "avg10" => avg10 = Hundredths::parse(value),
            "avg60" => avg60 = Hundredths::parse(value),
            "avg300" => avg300 = Hundredths::parse(value),
            _ => {}
        }
    }

    match (avg10, avg60, avg300) {
        (Some(avg10), Some(avg60), Some(avg300)) => Ok(PressureLine {
            avg10,
            avg60,
            avg300,
        }),
        _ => Err(ProcTextError::Malformed),
    }
}

/// A temperature in thousandths of a degree Celsius, as hwmon reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MilliCelsius(i32);

impl MilliCelsius {
    /// Wraps a raw hwmon reading.
    ///
    /// # Arguments
    ///
    /// * `raw` - thousandths of a degree Celsius
    #[must_use]
    pub const fn new(raw: i32) -> Self {
        Self(raw)
    }

    /// Whole degrees, rounded to the nearest.
    #[must_use]
    pub const fn rounded_degrees(self) -> i32 {
        let adjusted = if self.0 >= 0 {
            self.0.saturating_add(HALF_DEGREE)
        } else {
            self.0.saturating_sub(HALF_DEGREE)
        };
        adjusted / MILLIDEGREES_PER_DEGREE
    }
}

impl fmt::Display for MilliCelsius {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.pad(&format!("{}°C", self.rounded_degrees()))
    }
}

/// Parses an hwmon `temp*_input` file.
///
/// # Arguments
///
/// * `text` - contents of the file, an integer in thousandths of a degree
///
/// # Errors
///
/// Returns [`ProcTextError::Malformed`] when `text` is not an integer.
///
/// # Examples
///
/// ```
/// use borbatop_core::parse_millicelsius;
///
/// let temperature = parse_millicelsius("61250\n").map_err(|error| error.to_string())?;
/// assert_eq!(temperature.to_string(), "61°C");
/// # Ok::<(), String>(())
/// ```
pub fn parse_millicelsius(text: &str) -> Result<MilliCelsius, ProcTextError> {
    text.trim()
        .parse::<i32>()
        .map(MilliCelsius)
        .map_err(|_| ProcTextError::Malformed)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const MEMINFO: &str = "MemTotal:       16215896 kB\nMemFree:          665168 kB\n\
        MemAvailable:    4123708 kB\nBuffers:              16 kB\n\
        SwapTotal:      16215036 kB\nSwapFree:        8657452 kB\n";

    #[test]
    fn meminfo_computes_used_and_swap_used() {
        let info = parse_meminfo(MEMINFO).unwrap();
        assert_eq!(info.total(), Bytes::from_kib(16_215_896).unwrap());
        assert_eq!(
            info.used(),
            Bytes::from_kib(16_215_896 - 4_123_708).unwrap()
        );
        assert_eq!(
            info.swap_used(),
            Bytes::from_kib(16_215_036 - 8_657_452).unwrap()
        );
    }

    #[test]
    fn meminfo_reports_the_missing_or_invalid_field() {
        let without = MEMINFO.replace("MemAvailable:", "Nope:");
        assert_eq!(
            parse_meminfo(&without),
            Err(MemInfoError::Missing(MemField::Available))
        );
        let invalid = MEMINFO.replace("16215036 kB", "many kB");
        assert_eq!(
            parse_meminfo(&invalid),
            Err(MemInfoError::Invalid(MemField::SwapTotal))
        );
        assert_eq!(
            parse_meminfo("MemTotal: 18446744073709551615 kB\n"),
            Err(MemInfoError::Invalid(MemField::Total))
        );
    }

    #[test]
    fn loadavg_and_uptime() {
        let load = parse_loadavg("3.91 5.13 4.87 3/1488 50250\n").unwrap();
        assert_eq!(load.one().get(), 391);
        assert_eq!(load.fifteen().get(), 487);
        assert_eq!(parse_loadavg("x 1 2"), Err(ProcTextError::Malformed));
        assert_eq!(parse_loadavg("1.0 2.0"), Err(ProcTextError::Malformed));
        assert_eq!(parse_uptime("36365.04 1.0").unwrap().as_secs(), 36_365);
        assert_eq!(parse_uptime("soon"), Err(ProcTextError::Malformed));
        assert_eq!(parse_uptime(""), Err(ProcTextError::Malformed));
    }

    #[test]
    fn pressure_with_and_without_full() {
        let both = "some avg10=1.50 avg60=0.14 avg300=0.40 total=1\n\
                    full avg10=0.20 avg60=0.00 avg300=0.00 total=0\n";
        let parsed = parse_pressure(both).unwrap();
        assert_eq!(parsed.some().avg10().get(), 150);
        assert_eq!(parsed.full().unwrap().avg10().get(), 20);

        let only_some = "some avg10=0.00 avg60=0.00 avg300=0.00 total=0\n";
        assert_eq!(parse_pressure(only_some).unwrap().full(), None);
        assert_eq!(parse_pressure(""), Err(ProcTextError::Malformed));
        assert_eq!(
            parse_pressure("some avg10=0.00\n"),
            Err(ProcTextError::Malformed)
        );
    }

    #[test]
    fn temperature_rounds_to_the_nearest_degree() {
        assert_eq!(parse_millicelsius("61250\n").unwrap().to_string(), "61°C");
        assert_eq!(parse_millicelsius("61500").unwrap().to_string(), "62°C");
        assert_eq!(parse_millicelsius("-1500").unwrap().to_string(), "-2°C");
        assert_eq!(parse_millicelsius("hot"), Err(ProcTextError::Malformed));
    }
}
