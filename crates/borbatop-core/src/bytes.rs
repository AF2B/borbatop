//! Memory amounts, in bytes, with human-readable formatting.

use std::error::Error;
use std::fmt;

use crate::process::Pages;

const KIB: u64 = 1_024;
const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
const TENTHS: u64 = 10;

/// An amount of memory or storage, in bytes.
///
/// Displays in binary units with one decimal digit, truncated (`1.5G`).
///
/// # Examples
///
/// ```
/// use borbatop_core::Bytes;
///
/// assert_eq!(Bytes::new(512).to_string(), "512B");
/// assert_eq!(Bytes::new(1_610_612_736).to_string(), "1.5G");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct Bytes(u64);

impl Bytes {
    /// Wraps a raw byte count.
    ///
    /// # Arguments
    ///
    /// * `raw` - number of bytes
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Converts a count of kibibytes, as printed by `/proc/meminfo`.
    ///
    /// # Arguments
    ///
    /// * `kib` - number of kibibytes
    ///
    /// # Returns
    ///
    /// `None` when the byte count exceeds 64 bits.
    #[must_use]
    pub const fn from_kib(kib: u64) -> Option<Self> {
        match kib.checked_mul(KIB) {
            Some(bytes) => Some(Self(bytes)),
            None => None,
        }
    }

    /// The raw byte count.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Difference that stops at zero instead of going negative.
    ///
    /// # Arguments
    ///
    /// * `other` - amount to subtract
    #[must_use]
    pub const fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }

    /// Share of `total` that this amount represents, in permille, clamped to 1000.
    ///
    /// # Arguments
    ///
    /// * `total` - the whole; zero yields zero
    #[must_use]
    pub fn permille_of(self, total: Self) -> u64 {
        self.0
            .saturating_mul(1_000)
            .checked_div(total.0)
            .map_or(0, |share| share.min(1_000))
    }
}

impl fmt::Display for Bytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut whole = self.0;
        let mut tenth = 0;
        let mut unit = 0_usize;

        while whole >= KIB && unit < UNITS.len().saturating_sub(1) {
            tenth = (whole % KIB).saturating_mul(TENTHS) / KIB;
            whole /= KIB;
            unit = unit.saturating_add(1);
        }

        let suffix = UNITS.get(unit).copied().unwrap_or("?");
        if unit == 0 {
            formatter.pad(&format!("{whole}{suffix}"))
        } else {
            formatter.pad(&format!("{whole}.{tenth}{suffix}"))
        }
    }
}

/// Size of a memory page on this machine, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageSize(u64);

/// The page size is zero, which no machine has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZeroPageSize;

impl fmt::Display for ZeroPageSize {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "page size cannot be zero")
    }
}

impl Error for ZeroPageSize {}

impl PageSize {
    /// Validates a page size.
    ///
    /// # Arguments
    ///
    /// * `bytes` - size of one page, as the kernel reports it
    ///
    /// # Errors
    ///
    /// Returns [`ZeroPageSize`] when `bytes` is zero.
    pub const fn new(bytes: u64) -> Result<Self, ZeroPageSize> {
        if bytes == 0 {
            Err(ZeroPageSize)
        } else {
            Ok(Self(bytes))
        }
    }

    /// Memory covered by `pages` pages of this size.
    ///
    /// # Arguments
    ///
    /// * `pages` - number of pages
    ///
    /// # Returns
    ///
    /// `None` when the product exceeds 64 bits.
    #[must_use]
    pub const fn bytes_in(self, pages: Pages) -> Option<Bytes> {
        match pages.get().checked_mul(self.0) {
            Some(bytes) => Some(Bytes(bytes)),
            None => None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn formats_each_unit() {
        assert_eq!(Bytes::new(0).to_string(), "0B");
        assert_eq!(Bytes::new(1_023).to_string(), "1023B");
        assert_eq!(Bytes::new(1_024).to_string(), "1.0K");
        assert_eq!(Bytes::new(1_536).to_string(), "1.5K");
        assert_eq!(Bytes::new(1_048_576).to_string(), "1.0M");
        assert_eq!(Bytes::new(17_179_869_184).to_string(), "16.0G");
        assert_eq!(Bytes::new(u64::MAX).to_string(), "16383.9P");
    }

    #[test]
    fn honors_width() {
        assert_eq!(format!("{:>7}", Bytes::new(2_048)), "   2.0K");
    }

    #[test]
    fn converts_kibibytes_with_overflow_guard() {
        assert_eq!(Bytes::from_kib(2), Some(Bytes::new(2_048)));
        assert_eq!(Bytes::from_kib(u64::MAX), None);
    }

    #[test]
    fn share_is_clamped_and_zero_safe() {
        assert_eq!(Bytes::new(1).permille_of(Bytes::new(4)), 250);
        assert_eq!(Bytes::new(9).permille_of(Bytes::new(4)), 1_000);
        assert_eq!(Bytes::new(9).permille_of(Bytes::new(0)), 0);
    }

    #[test]
    fn page_size_rejects_zero_and_scales_pages() {
        assert_eq!(PageSize::new(0), Err(ZeroPageSize));
        let size = PageSize::new(4_096).unwrap();
        let pages = crate::process::parse_process_stat(
            "1 (a) S 0 1 1 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 1 1 25 0",
        )
        .unwrap()
        .resident_pages();
        assert_eq!(size.bytes_in(pages), Some(Bytes::new(102_400)));
    }

    #[test]
    fn saturating_sub_stops_at_zero() {
        assert_eq!(Bytes::new(3).saturating_sub(Bytes::new(5)), Bytes::new(0));
    }
}
