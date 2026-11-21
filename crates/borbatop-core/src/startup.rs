//! Boot time per unit: a parser for `systemd-analyze blame` and for systemd's duration text.

use std::time::Duration;

const NANOS_PER_MICRO: u128 = 1_000;
const NANOS_PER_MILLI: u128 = 1_000_000;
const NANOS_PER_SECOND: u128 = 1_000_000_000;
const NANOS_PER_MINUTE: u128 = 60 * NANOS_PER_SECOND;
const NANOS_PER_HOUR: u128 = 60 * NANOS_PER_MINUTE;
const NANOS_PER_DAY: u128 = 24 * NANOS_PER_HOUR;
const DECIMAL_BASE: u128 = 10;
const FRACTION_DIGITS_KEPT: usize = 9;

/// One unit and how long it took to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitStartup {
    duration: Duration,
    unit: String,
}

impl UnitStartup {
    /// Time the unit took to initialize.
    #[must_use]
    pub const fn duration(&self) -> Duration {
        self.duration
    }

    /// Unit name.
    #[must_use]
    pub fn unit(&self) -> &str {
        &self.unit
    }
}

/// The parsed output of `systemd-analyze blame`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupBlame {
    units: Vec<UnitStartup>,
    skipped: usize,
}

impl StartupBlame {
    /// Units from the slowest to the fastest, as the command prints them.
    #[must_use]
    pub fn units(&self) -> &[UnitStartup] {
        &self.units
    }

    /// Lines that were not understood, counted instead of dropped silently.
    #[must_use]
    pub const fn skipped(&self) -> usize {
        self.skipped
    }
}

/// Parses the output of `systemd-analyze blame`.
///
/// # Arguments
///
/// * `text` - standard output of the command
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use borbatop_core::parse_startup_blame;
///
/// let blame = parse_startup_blame("1min 2.5s slow.service\n  345ms fast.service\n");
/// assert_eq!(blame.units().first().map(|unit| unit.duration()), Some(Duration::from_millis(62_500)));
/// assert_eq!(blame.units().get(1).map(|unit| unit.unit()), Some("fast.service"));
/// ```
#[must_use]
pub fn parse_startup_blame(text: &str) -> StartupBlame {
    let mut units = Vec::new();
    let mut skipped = 0_usize;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        match parse_blame_line(line) {
            Some(startup) => units.push(startup),
            None => skipped = skipped.saturating_add(1),
        }
    }
    StartupBlame { units, skipped }
}

fn parse_blame_line(line: &str) -> Option<UnitStartup> {
    let trimmed = line.trim();
    let split = trimmed.rfind(char::is_whitespace)?;
    let (time, unit) = trimmed.split_at(split);
    Some(UnitStartup {
        duration: parse_systemd_duration(time)?,
        unit: unit.trim().to_owned(),
    })
}

/// Parses durations as systemd prints them: `1min 2.345s`, `345ms`, `12us`, `2h 5min`.
///
/// # Arguments
///
/// * `text` - one or more `<number><unit>` parts separated by spaces; units are `us`, `ms`,
///   `s`, `min`, `h` and `d`
///
/// # Returns
///
/// `None` for an unknown unit, a malformed number, empty text or a value that overflows.
#[must_use]
pub fn parse_systemd_duration(text: &str) -> Option<Duration> {
    let mut total = 0_u128;
    let mut parts = 0_usize;
    for part in text.split_ascii_whitespace() {
        total = total.checked_add(part_nanos(part)?)?;
        parts = parts.saturating_add(1);
    }
    if parts == 0 {
        return None;
    }
    Some(Duration::from_nanos(u64::try_from(total).ok()?))
}

fn part_nanos(part: &str) -> Option<u128> {
    let split = part.find(|character: char| !(character.is_ascii_digit() || character == '.'))?;
    let (number, unit) = part.split_at(split);
    let unit_nanos = match unit {
        "us" => NANOS_PER_MICRO,
        "ms" => NANOS_PER_MILLI,
        "s" => NANOS_PER_SECOND,
        "min" => NANOS_PER_MINUTE,
        "h" => NANOS_PER_HOUR,
        "d" => NANOS_PER_DAY,
        _ => return None,
    };

    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    if whole.is_empty() || !fraction.chars().all(|digit| digit.is_ascii_digit()) {
        return None;
    }
    let fraction = fraction.get(..fraction.len().min(FRACTION_DIGITS_KEPT))?;
    let scale = DECIMAL_BASE.checked_pow(u32::try_from(fraction.len()).ok()?)?;
    let fraction_value = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u128>().ok()?
    };
    let scaled = whole
        .parse::<u128>()
        .ok()?
        .checked_mul(scale)?
        .checked_add(fraction_value)?;
    scaled.checked_mul(unit_nanos)?.checked_div(scale)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn durations_in_every_unit_systemd_prints() {
        assert_eq!(
            parse_systemd_duration("12us"),
            Some(Duration::from_micros(12))
        );
        assert_eq!(
            parse_systemd_duration("345ms"),
            Some(Duration::from_millis(345))
        );
        assert_eq!(
            parse_systemd_duration("2.345s"),
            Some(Duration::from_millis(2_345))
        );
        assert_eq!(
            parse_systemd_duration("1min 2.5s"),
            Some(Duration::from_millis(62_500))
        );
        assert_eq!(
            parse_systemd_duration("2h 5min"),
            Some(Duration::from_mins(125))
        );
        assert_eq!(parse_systemd_duration("1d"), Some(Duration::from_hours(24)));
    }

    #[test]
    fn malformed_durations_are_refused() {
        for text in [
            "", "  ", "5", "s", "5parsecs", "1..2s", "-3s", "1,5s", "1.s5", ".5s",
        ] {
            assert_eq!(parse_systemd_duration(text), None, "{text:?}");
        }
        assert_eq!(parse_systemd_duration("99999999999999999999d"), None);
    }

    #[test]
    fn a_trailing_dot_or_more_digits_than_nanoseconds_do_not_panic() {
        assert_eq!(parse_systemd_duration("1.s"), Some(Duration::from_secs(1)));
        assert_eq!(
            parse_systemd_duration("1.1234567891234s"),
            Some(Duration::from_nanos(1_123_456_789))
        );
    }

    #[test]
    fn blame_keeps_order_and_counts_what_it_cannot_read() {
        let blame = parse_startup_blame(
            "  5.2s NetworkManager-wait-online.service\n 1min 3s plymouth.service\n  45ms a.service\nnot a line\n\n",
        );
        let names: Vec<&str> = blame.units().iter().map(UnitStartup::unit).collect();
        assert_eq!(
            names,
            [
                "NetworkManager-wait-online.service",
                "plymouth.service",
                "a.service"
            ]
        );
        assert_eq!(
            blame.units().get(1).unwrap().duration(),
            Duration::from_secs(63)
        );
        assert_eq!(blame.skipped(), 1);
    }
}
