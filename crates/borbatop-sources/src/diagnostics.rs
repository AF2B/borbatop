//! Boot time and the system journal, through `systemd-analyze` and `journalctl`.

use borbatop_core::{StartupBlame, parse_startup_blame};

use crate::command::run;
use crate::error::CommandError;

/// How serious the journal lines to fetch must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalLevel {
    /// Emergency, alert, critical and error messages.
    Errors,
    /// Warnings only (errors are fetched separately).
    Warnings,
}

impl JournalLevel {
    /// The `journalctl --priority` argument.
    #[must_use]
    pub const fn priority(self) -> &'static str {
        match self {
            Self::Errors => "err",
            Self::Warnings => "warning..warning",
        }
    }
}

/// The summary line of `systemd-analyze time`: firmware, loader, kernel and userspace time.
///
/// # Errors
///
/// Returns [`CommandError`] when `systemd-analyze` cannot be started or fails.
pub fn startup_summary() -> Result<String, CommandError> {
    run("systemd-analyze", &["time", "--no-pager"], &[])
}

/// Every unit with the time it took to start, slowest first.
///
/// # Errors
///
/// Returns [`CommandError`] when `systemd-analyze` cannot be started or fails.
pub fn startup_blame() -> Result<StartupBlame, CommandError> {
    run("systemd-analyze", &["blame", "--no-pager"], &[]).map(|text| parse_startup_blame(&text))
}

/// The newest system journal lines of one level since the current boot.
///
/// The journal can hold personal data: show it to the person at the keyboard, never log it.
///
/// # Arguments
///
/// * `level` - which messages to fetch
/// * `lines` - how many of the latest lines to return
///
/// # Errors
///
/// Returns [`CommandError`] when `journalctl` cannot be started or fails (for example, the user
/// may not read the system journal).
pub fn system_journal(level: JournalLevel, lines: u16) -> Result<String, CommandError> {
    let count = lines.to_string();
    run(
        "journalctl",
        &[
            "--boot",
            "--priority",
            level.priority(),
            "--lines",
            &count,
            "--no-pager",
            "--output",
            "short",
        ],
        &[],
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::path::Path;

    use super::*;

    fn systemd_booted() -> bool {
        Path::new("/run/systemd/system").exists()
    }

    #[test]
    fn priorities_match_journalctl_syntax() {
        assert_eq!(JournalLevel::Errors.priority(), "err");
        assert_eq!(JournalLevel::Warnings.priority(), "warning..warning");
    }

    #[test]
    fn this_boot_has_a_startup_summary_and_a_blame_list() {
        if !systemd_booted() {
            return; // not booted with systemd (container, chroot)
        }
        let Ok(summary) = startup_summary() else {
            return; // systemd-analyze may be absent or refuse in minimal setups
        };
        assert!(summary.contains("Startup finished") || summary.contains("Startup"));
        let blame = startup_blame().unwrap();
        assert_ne!(blame.units(), []);
        assert_eq!(blame.skipped(), 0);
    }
}
