//! Who currently holds a PID: the check that keeps actions off a reused PID.

use std::fs;
use std::io;

use borbatop_core::{Pid, ProcessId, parse_process_stat};

const PROC_ROOT: &str = "/proc";
const STAT_FILE: &str = "stat";

/// Why the identity behind a PID could not be read.
#[derive(Debug)]
pub(crate) enum IdentityError {
    /// No process has this PID any more.
    Gone,
    /// The stat file exists but cannot be read or understood.
    Unreadable(io::Error),
}

/// Whether the test run has root's privileges (a CI container usually does): tests about what
/// an unprivileged user may not do must be skipped then.
#[cfg(test)]
pub(crate) fn running_as_root() -> bool {
    fs::read_to_string("/proc/self/status")
        .is_ok_and(|status| status.lines().any(|line| line.starts_with("Uid:\t0\t")))
}

/// Identity (PID plus start time) of whatever process currently holds `pid`.
///
/// # Arguments
///
/// * `pid` - the process identifier to look up
///
/// # Errors
///
/// * [`IdentityError::Gone`] when `/proc/<pid>/stat` does not exist.
/// * [`IdentityError::Unreadable`] for any other read or parse failure.
pub(crate) fn current_identity(pid: Pid) -> Result<ProcessId, IdentityError> {
    let path = format!("{PROC_ROOT}/{pid}/{STAT_FILE}");
    let bytes = fs::read(&path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            IdentityError::Gone
        } else {
            IdentityError::Unreadable(error)
        }
    })?;
    parse_process_stat(&String::from_utf8_lossy(&bytes))
        .map(|sample| sample.id())
        .map_err(|error| {
            IdentityError::Unreadable(io::Error::new(io::ErrorKind::InvalidData, error))
        })
}
