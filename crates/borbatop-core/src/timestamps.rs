//! Timestamps as they appear in the files and feeds the program reads.

use crate::clock::unix_from_civil;

const SECONDS_PER_MINUTE: i64 = 60;
const SECONDS_PER_HOUR: i64 = 3_600;
const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const OFFSET_DIGITS: usize = 4;

/// Seconds since the Unix epoch of an RFC 822 date as RSS feeds write it
/// (`Wed, 21 Oct 2026 19:29:36 +0000`); the weekday is optional.
///
/// # Arguments
///
/// * `text` - the date, with a numeric offset or `GMT`/`UTC`
///
/// # Examples
///
/// ```
/// use borbatop_core::parse_rfc822;
///
/// assert_eq!(parse_rfc822("Wed, 21 Oct 2026 19:29:36 +0000"), Some(1_792_610_976));
/// assert_eq!(parse_rfc822("21 Oct 2026 16:29:36 -0300"), Some(1_792_610_976));
/// ```
#[must_use]
pub fn parse_rfc822(text: &str) -> Option<u64> {
    let text = text.trim();
    let text = text.split_once(',').map_or(text, |(_, rest)| rest).trim();
    let mut parts = text.split_ascii_whitespace();
    let day: u64 = parts.next()?.parse().ok()?;
    let month = month_number(parts.next()?)?;
    let year: u64 = parts.next()?.parse().ok()?;
    let (hour, minute, second) = clock_time(parts.next()?)?;
    let offset = offset_seconds(parts.next()?)?;
    apply_offset(
        unix_from_civil(year, month, day, hour, minute, second)?,
        offset,
    )
}

/// Seconds since the Unix epoch of a timestamp in `pacman.log`
/// (`2026-10-27T00:06:21-0300`).
///
/// # Arguments
///
/// * `text` - the date and time with a numeric offset such as `-0300`
///
/// # Examples
///
/// ```
/// use borbatop_core::parse_log_timestamp;
///
/// assert_eq!(parse_log_timestamp("2026-10-27T00:06:21-0300"), Some(1_793_070_381));
/// ```
#[must_use]
pub fn parse_log_timestamp(text: &str) -> Option<u64> {
    let (date, rest) = text.trim().split_once('T')?;
    let split = rest.find(['+', '-'])?;
    let (time, offset) = rest.split_at(split);

    let mut fields = date.split('-');
    let year: u64 = fields.next()?.parse().ok()?;
    let month: u64 = fields.next()?.parse().ok()?;
    let day: u64 = fields.next()?.parse().ok()?;
    if fields.next().is_some() {
        return None;
    }
    let (hour, minute, second) = clock_time(time)?;
    apply_offset(
        unix_from_civil(year, month, day, hour, minute, second)?,
        offset_seconds(offset)?,
    )
}

fn month_number(name: &str) -> Option<u64> {
    let position = MONTH_NAMES
        .iter()
        .position(|month| month.eq_ignore_ascii_case(name))?;
    u64::try_from(position).ok()?.checked_add(1)
}

fn clock_time(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.split(':');
    let hour = parts.next()?.parse().ok()?;
    let minute = parts.next()?.parse().ok()?;
    let second = parts.next().map_or(Some(0), |second| second.parse().ok())?;
    parts.next().is_none().then_some((hour, minute, second))
}

/// `+0000`, `-0300`, `GMT` or `UTC`, in seconds east of UTC.
fn offset_seconds(text: &str) -> Option<i64> {
    if text.eq_ignore_ascii_case("GMT") || text.eq_ignore_ascii_case("UTC") || text == "Z" {
        return Some(0);
    }
    let sign = match text.chars().next()? {
        '+' => 1_i64,
        '-' => -1_i64,
        _ => return None,
    };
    let digits = text.get(1..)?;
    if digits.len() != OFFSET_DIGITS || !digits.chars().all(|digit| digit.is_ascii_digit()) {
        return None;
    }
    let hours: i64 = digits.get(..2)?.parse().ok()?;
    let minutes: i64 = digits.get(2..)?.parse().ok()?;
    if minutes > 59 {
        return None;
    }
    let magnitude = hours
        .checked_mul(SECONDS_PER_HOUR)?
        .checked_add(minutes.checked_mul(SECONDS_PER_MINUTE)?)?;
    sign.checked_mul(magnitude)
}

/// Local time minus its offset east of UTC is UTC.
fn apply_offset(local: u64, offset_east: i64) -> Option<u64> {
    local.checked_add_signed(offset_east.checked_neg()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc822_dates_from_feeds() {
        assert_eq!(
            parse_rfc822("Wed, 21 Oct 2026 19:29:36 +0000"),
            Some(1_792_610_976)
        );
        assert_eq!(
            parse_rfc822("21 Oct 2026 19:29:36 GMT"),
            Some(1_792_610_976)
        );
        assert_eq!(
            parse_rfc822("Wed, 21 Oct 2026 16:29:36 -0300"),
            Some(1_792_610_976)
        );
        assert_eq!(
            parse_rfc822("Thu, 29 Feb 2024 12:00 +0000"),
            Some(1_709_208_000)
        );
        assert_eq!(
            parse_rfc822("Thu, 29 Feb 2024 15:30:00 +0330"),
            Some(1_709_208_000)
        );
    }

    #[test]
    fn broken_rfc822_dates_are_refused() {
        for text in [
            "",
            "Wed, 21 Foo 2026 19:29:36 +0000",
            "Wed, 31 Feb 2026 19:29:36 +0000",
            "Wed, 21 Oct 2026 25:29:36 +0000",
            "Wed, 21 Oct 2026 19:29:36",
            "Wed, 21 Oct 2026 19:29:36 +00",
            "Wed, 21 Oct 2026 19:29:36 +0099",
            "yesterday",
        ] {
            assert_eq!(parse_rfc822(text), None, "{text:?}");
        }
    }

    #[test]
    fn pacman_log_timestamps_apply_their_offset() {
        assert_eq!(
            parse_log_timestamp("2026-10-27T00:06:21-0300"),
            Some(1_793_070_381)
        );
        assert_eq!(
            parse_log_timestamp("2026-10-07T11:59:58-0300"),
            Some(1_791_385_198)
        );
        assert_eq!(
            parse_log_timestamp("2026-10-27T03:06:21+0000"),
            Some(1_793_070_381)
        );
        assert_eq!(
            parse_log_timestamp("2026-10-27T05:06:21+0200"),
            Some(1_793_070_381)
        );
    }

    #[test]
    fn broken_log_timestamps_are_refused() {
        for text in [
            "",
            "2026-10-27",
            "2026-10-27 00:06:21-0300",
            "2026-13-27T00:06:21-0300",
            "2026-10-27T00:06:21",
            "2026-10-27-1T00:06:21-0300",
        ] {
            assert_eq!(parse_log_timestamp(text), None, "{text:?}");
        }
    }
}
