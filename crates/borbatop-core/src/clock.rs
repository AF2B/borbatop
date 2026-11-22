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

/// Seconds since the Unix epoch of a calendar date and time (UTC), or `None` when a field is out
/// of range or the date is before 1970.
///
/// # Arguments
///
/// * `year` - full year, 1970 or later
/// * `month` - 1 to 12
/// * `day` - 1 to 31, and valid for the month
/// * `hour` - 0 to 23
/// * `minute` - 0 to 59
/// * `second` - 0 to 60 (a leap second counts as the last second of the minute)
#[must_use]
pub fn unix_from_civil(
    year: u64,
    month: u64,
    day: u64,
    hour: u64,
    minute: u64,
    second: u64,
) -> Option<u64> {
    if !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let days = days_from_civil(year, month, day)?;
    // The reverse trip rejects days the month does not have (31 February).
    if civil_from_days(days) != (year, month, day) {
        return None;
    }
    days.checked_mul(SECONDS_PER_DAY)?
        .checked_add(hour.checked_mul(SECONDS_PER_HOUR)?)?
        .checked_add(minute.checked_mul(SECONDS_PER_MINUTE)?)?
        .checked_add(second)
}

/// Days since 1970-01-01 of a civil date; the inverse of [`civil_from_days`].
fn days_from_civil(year: u64, month: u64, day: u64) -> Option<u64> {
    // The civil year of this algorithm starts in March, so January and February belong to the
    // previous one.
    let shifted_year = if month <= FEBRUARY {
        year.checked_sub(1)?
    } else {
        year
    };
    let era = shifted_year.div_euclid(YEARS_PER_ERA);
    let year_of_era = shifted_year.rem_euclid(YEARS_PER_ERA);
    let month_index = if month > FEBRUARY {
        month.checked_sub(FIRST_MONTH_OF_CIVIL_YEAR)?
    } else {
        month.checked_add(MONTHS_BEFORE_MARCH_SHIFT)?
    };
    let day_of_year = month_index
        .checked_mul(MONTH_STEP_NUMERATOR)?
        .checked_add(MONTH_STEP_OFFSET)?
        .div_euclid(MONTH_STEP_DENOMINATOR)
        .checked_add(day.checked_sub(1)?)?;
    let day_of_era = year_of_era
        .checked_mul(DAYS_PER_YEAR)?
        .checked_add(year_of_era.div_euclid(4))?
        .checked_sub(year_of_era.div_euclid(100))?
        .checked_add(day_of_year)?;
    era.checked_mul(DAYS_PER_ERA)?
        .checked_add(day_of_era)?
        .checked_sub(DAYS_TO_UNIX_EPOCH)
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
    fn civil_dates_convert_back_to_unix_seconds() {
        for (fields, seconds) in [
            ((1970, 1, 1, 0, 0, 0), 0),
            ((2000, 2, 29, 0, 0, 0), 951_782_400),
            ((2001, 9, 9, 1, 46, 40), 1_000_000_000),
            ((2024, 2, 29, 12, 0, 0), 1_709_208_000),
            ((2024, 12, 31, 23, 59, 59), 1_735_689_599),
            ((2026, 11, 21, 21, 7, 4), 1_795_295_224),
        ] {
            let (y, mo, d, h, mi, s) = fields;
            assert_eq!(
                unix_from_civil(y, mo, d, h, mi, s),
                Some(seconds),
                "{fields:?}"
            );
        }
    }

    #[test]
    fn impossible_dates_are_refused() {
        for (y, mo, d, h, mi, s) in [
            (2026, 2, 29, 0, 0, 0),
            (2026, 4, 31, 0, 0, 0),
            (2026, 13, 1, 0, 0, 0),
            (2026, 0, 1, 0, 0, 0),
            (2026, 1, 0, 0, 0, 0),
            (2026, 1, 1, 24, 0, 0),
            (1969, 12, 31, 0, 0, 0),
            (2026, 1, 1, 0, 60, 0),
        ] {
            assert_eq!(
                unix_from_civil(y, mo, d, h, mi, s),
                None,
                "{y}-{mo}-{d} {h}:{mi}:{s}"
            );
        }
    }

    #[test]
    fn the_round_trip_holds_for_every_day_of_a_few_years() {
        let mut days = 0_u64;
        for year in 2023..=2026_u64 {
            for month in 1..=12_u64 {
                for day in 1..=31_u64 {
                    if let Some(seconds) = unix_from_civil(year, month, day, 0, 0, 0) {
                        let back = UtcDateTime::from_unix_seconds(seconds).file_stamp();
                        assert_eq!(back, format!("{year:04}{month:02}{day:02}-000000"));
                        days += 1;
                    }
                }
            }
        }
        assert_eq!(days, 1_461, "2023 to 2026 has one leap year");
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
