//! Calendar date and time of day from a Unix timestamp (UTC), without a date crate.

use std::fmt;

const SECONDS_PER_MINUTE: u64 = 60;
const SECONDS_PER_HOUR: u64 = 3_600;
const SECONDS_PER_DAY: u64 = 86_400;
/// Days from 0000-03-01 to 1970-01-01 (the civil calendar of Howard Hinnant's algorithm).
const DAYS_TO_UNIX_EPOCH: u64 = 719_468;
const DAYS_PER_ERA: u64 = 146_097;
const YEARS_PER_ERA: u64 = 400;
const DAYS_PER_YEAR: u64 = 365;
const DAYS_PER_4_YEARS: u64 = 1_460;
const DAYS_PER_100_YEARS: u64 = 36_524;
const DAYS_PER_400_MINUS_ONE: u64 = 146_096;
const MONTH_STEP_NUMERATOR: u64 = 153;
const MONTH_STEP_OFFSET: u64 = 2;
const MONTH_STEP_DENOMINATOR: u64 = 5;
const FIRST_MONTH_OF_CIVIL_YEAR: u64 = 3;
const MONTHS_BEFORE_MARCH_SHIFT: u64 = 9;
const MARCH_SHIFT_LIMIT: u64 = 10;
const FEBRUARY: u64 = 2;

/// A moment in UTC, to the second.
///
/// # Examples
///
/// ```
/// use borbatop_core::UtcDateTime;
///
/// let moment = UtcDateTime::from_unix_seconds(1_000_000_000);
/// assert_eq!(moment.file_stamp(), "20010909-014640");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UtcDateTime {
    year: u64,
    month: u64,
    day: u64,
    hour: u64,
    minute: u64,
    second: u64,
}

impl UtcDateTime {
    /// The moment `seconds` after 1970-01-01 00:00:00 UTC.
    ///
    /// # Arguments
    ///
    /// * `seconds` - seconds since the Unix epoch
    #[must_use]
    pub fn from_unix_seconds(seconds: u64) -> Self {
        let days = seconds.div_euclid(SECONDS_PER_DAY);
        let in_day = seconds.rem_euclid(SECONDS_PER_DAY);
        let (year, month, day) = civil_from_days(days);
        Self {
            year,
            month,
            day,
            hour: in_day.div_euclid(SECONDS_PER_HOUR),
            minute: in_day
                .rem_euclid(SECONDS_PER_HOUR)
                .div_euclid(SECONDS_PER_MINUTE),
            second: in_day.rem_euclid(SECONDS_PER_MINUTE),
        }
    }

    /// `YYYYMMDD-HHMMSS`, safe to use in a file name and sortable.
    #[must_use]
    pub fn file_stamp(&self) -> String {
        format!(
            "{:04}{:02}{:02}-{:02}{:02}{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

impl fmt::Display for UtcDateTime {
    /// `YYYY-MM-DD HH:MM:SS UTC`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

/// Year, month (1-12) and day (1-31) of a count of days since 1970-01-01.
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let shifted = days.saturating_add(DAYS_TO_UNIX_EPOCH);
    let era = shifted.div_euclid(DAYS_PER_ERA);
    let day_of_era = shifted.rem_euclid(DAYS_PER_ERA);
    let year_of_era = day_of_era
        .saturating_sub(day_of_era.div_euclid(DAYS_PER_4_YEARS))
        .saturating_add(day_of_era.div_euclid(DAYS_PER_100_YEARS))
        .saturating_sub(day_of_era.div_euclid(DAYS_PER_400_MINUS_ONE))
        .div_euclid(DAYS_PER_YEAR);
    let year = year_of_era.saturating_add(era.saturating_mul(YEARS_PER_ERA));
    let day_of_year = day_of_era.saturating_sub(
        DAYS_PER_YEAR
            .saturating_mul(year_of_era)
            .saturating_add(year_of_era.div_euclid(4))
            .saturating_sub(year_of_era.div_euclid(100)),
    );
    let month_index = day_of_year
        .saturating_mul(5)
        .saturating_add(MONTH_STEP_OFFSET)
        .div_euclid(MONTH_STEP_NUMERATOR);
    let day = day_of_year
        .saturating_sub(
            month_index
                .saturating_mul(MONTH_STEP_NUMERATOR)
                .saturating_add(MONTH_STEP_OFFSET)
                .div_euclid(MONTH_STEP_DENOMINATOR),
        )
        .saturating_add(1);
    let month = if month_index < MARCH_SHIFT_LIMIT {
        month_index.saturating_add(FIRST_MONTH_OF_CIVIL_YEAR)
    } else {
        month_index.saturating_sub(MONTHS_BEFORE_MARCH_SHIFT)
    };
    // January and February belong to the next civil year in this March-based calendar.
    let year = if month <= FEBRUARY {
        year.saturating_add(1)
    } else {
        year
    };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_instants_match_the_calendar() {
        for (seconds, stamp) in [
            (0, "19700101-000000"),
            (951_782_400, "20000229-000000"),
            (1_000_000_000, "20010909-014640"),
            (1_709_208_000, "20240229-120000"),
            (1_735_689_599, "20241231-235959"),
            (1_795_295_224, "20261121-210704"),
            (4_102_444_800, "21000101-000000"),
        ] {
            assert_eq!(UtcDateTime::from_unix_seconds(seconds).file_stamp(), stamp);
        }
    }

    #[test]
    fn display_is_readable_and_zero_padded() {
        let text = UtcDateTime::from_unix_seconds(1_000_000_000).to_string();
        assert_eq!(text, "2001-09-09 01:46:40 UTC");
    }

    #[test]
    fn every_day_of_a_leap_and_a_common_year_is_valid() {
        let day = SECONDS_PER_DAY;
        let start_2024 = 1_704_067_200_u64; // 2024-01-01
        let mut seen_29_feb = false;
        for offset in 0..366 {
            let moment = UtcDateTime::from_unix_seconds(start_2024 + offset * day);
            assert_eq!(moment.year, 2024);
            assert!((1..=12).contains(&moment.month));
            assert!((1..=31).contains(&moment.day));
            seen_29_feb |= moment.month == 2 && moment.day == 29;
        }
        assert!(seen_29_feb);
        let first_of_2025 = UtcDateTime::from_unix_seconds(start_2024 + 366 * day);
        assert_eq!(first_of_2025.file_stamp(), "20250101-000000");
    }

    #[test]
    fn the_largest_timestamp_does_not_panic() {
        let _moment = UtcDateTime::from_unix_seconds(u64::MAX);
    }
}
