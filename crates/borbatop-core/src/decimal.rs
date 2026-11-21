//! Fixed-point numbers with two decimal digits, parsed from kernel text without floats.

use std::fmt;

const SCALE: u32 = 100;
const DECIMAL_DIGITS: usize = 2;

/// A non-negative number with two decimal digits, stored as hundredths (`1.42` is 142).
///
/// # Examples
///
/// ```
/// use borbatop_core::Hundredths;
///
/// let load = Hundredths::parse("3.91").ok_or("not a decimal")?;
/// assert_eq!(load.get(), 391);
/// assert_eq!(load.to_string(), "3.91");
/// # Ok::<(), &str>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Hundredths(u32);

impl Hundredths {
    /// Wraps a raw count of hundredths.
    ///
    /// # Arguments
    ///
    /// * `raw` - the value multiplied by 100
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw count of hundredths.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Parses `"3.91"`, `"3"` or `"3.9"`; digits past the second decimal are dropped.
    ///
    /// # Arguments
    ///
    /// * `token` - decimal text as printed by procfs
    ///
    /// # Returns
    ///
    /// `None` when `token` is not a non-negative decimal or does not fit in 32 bits.
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        let (whole, fraction) = token.split_once('.').unwrap_or((token, ""));
        if !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }

        let whole = whole.parse::<u32>().ok()?;
        let mut digits = fraction.chars().chain(std::iter::repeat('0'));
        let mut hundredths = 0_u32;
        for _ in 0..DECIMAL_DIGITS {
            let digit = digits.next()?.to_digit(10)?;
            hundredths = hundredths.checked_mul(10)?.checked_add(digit)?;
        }

        whole.checked_mul(SCALE)?.checked_add(hundredths).map(Self)
    }
}

impl fmt::Display for Hundredths {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let whole = self.0 / SCALE;
        let fraction = self.0 % SCALE;
        formatter.pad(&format!("{whole}.{fraction:02}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_prints_round_trip() {
        assert_eq!(Hundredths::parse("3.91"), Some(Hundredths::new(391)));
        assert_eq!(Hundredths::parse("0.00"), Some(Hundredths::new(0)));
        assert_eq!(Hundredths::parse("12"), Some(Hundredths::new(1_200)));
        assert_eq!(Hundredths::parse("0.5"), Some(Hundredths::new(50)));
        assert_eq!(Hundredths::parse("1.239"), Some(Hundredths::new(123)));
        assert_eq!(Hundredths::new(391).to_string(), "3.91");
        assert_eq!(Hundredths::new(5).to_string(), "0.05");
    }

    #[test]
    fn rejects_garbage_and_overflow() {
        assert_eq!(Hundredths::parse(""), None);
        assert_eq!(Hundredths::parse("x.1"), None);
        assert_eq!(Hundredths::parse("1.x"), None);
        assert_eq!(Hundredths::parse("-1.0"), None);
        assert_eq!(Hundredths::parse("99999999999"), None);
        assert_eq!(Hundredths::parse("99999999.99"), None);
    }
}
