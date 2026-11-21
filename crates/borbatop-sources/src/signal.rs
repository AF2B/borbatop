//! Sending signals to processes through safe syscall wrappers, without PID-reuse races.

use std::error::Error;
use std::fmt;
use std::fs;
use std::io;

use borbatop_core::{Pid, ProcessId, parse_process_stat};
use rustix::io::Errno;
use rustix::process::{
    Pid as RawPid, PidfdFlags, Signal as RawSignal, pidfd_open, pidfd_send_signal,
};

const PROC_ROOT: &str = "/proc";
const STAT_FILE: &str = "stat";

/// The signals a task manager offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// `SIGTERM`: ask the process to finish cleanly.
    Terminate,
    /// `SIGHUP`: ask the process to reload its configuration.
    Hangup,
    /// `SIGKILL`: end the process immediately, without cleanup.
    Kill,
}

impl Signal {
    /// Name as `kill -l` prints it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Terminate => "SIGTERM",
            Self::Hangup => "SIGHUP",
            Self::Kill => "SIGKILL",
        }
    }

    const fn raw(self) -> RawSignal {
        match self {
            Self::Terminate => RawSignal::TERM,
            Self::Hangup => RawSignal::HUP,
            Self::Kill => RawSignal::KILL,
        }
    }
}

/// Why a signal was not delivered.
#[derive(Debug)]
pub enum SignalError {
    /// The process belongs to another user (`EPERM`).
    NotPermitted,
    /// The process exited, or its PID now belongs to a different process.
    NoSuchProcess,
    /// The PID is zero or does not fit the kernel's PID type.
    InvalidPid(Pid),
    /// Any other OS error.
    Os(io::Error),
}

impl fmt::Display for SignalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPermitted => write!(formatter, "operation not permitted (EPERM)"),
            Self::NoSuchProcess => write!(formatter, "process already gone (ESRCH)"),
            Self::InvalidPid(pid) => write!(formatter, "invalid PID {pid}"),
            Self::Os(error) => error.fmt(formatter),
        }
    }
}

impl Error for SignalError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Os(error) => Some(error),
            Self::NotPermitted | Self::NoSuchProcess | Self::InvalidPid(_) => None,
        }
    }
}

fn from_errno(errno: Errno) -> SignalError {
    if errno == Errno::PERM {
        SignalError::NotPermitted
    } else if errno == Errno::SRCH {
        SignalError::NoSuchProcess
    } else {
        SignalError::Os(io::Error::from_raw_os_error(errno.raw_os_error()))
    }
}

/// Sends `signal` to the process identified by `id`, and to no other.
///
/// A bare PID is not enough: between the moment a person picks a process and the moment the
/// signal leaves, the process may exit and the kernel may hand its PID to a stranger. This
/// function pins the process with a pidfd, then checks that the PID still has the start time
/// in `id`, and only then signals the pidfd.
///
/// # Arguments
///
/// * `id` - PID plus start time of the process to signal; zero PIDs are rejected so a process
///   group is never hit by accident
/// * `signal` - signal to deliver
///
/// # Errors
///
/// * [`SignalError::InvalidPid`] when the PID is zero or exceeds the kernel's PID range.
/// * [`SignalError::NotPermitted`] when the process belongs to another user.
/// * [`SignalError::NoSuchProcess`] when the process exited or its PID was reused.
/// * [`SignalError::Os`] for any other failure.
pub fn send_signal(id: ProcessId, signal: Signal) -> Result<(), SignalError> {
    let pid = id.pid();
    let raw_pid = i32::try_from(pid.get())
        .ok()
        .and_then(RawPid::from_raw)
        .ok_or(SignalError::InvalidPid(pid))?;

    let pidfd = pidfd_open(raw_pid, PidfdFlags::empty()).map_err(from_errno)?;
    if current_identity(pid)? != id {
        return Err(SignalError::NoSuchProcess);
    }
    pidfd_send_signal(&pidfd, signal.raw()).map_err(from_errno)
}

/// Identity of whatever process currently holds `pid`.
fn current_identity(pid: Pid) -> Result<ProcessId, SignalError> {
    let path = format!("{PROC_ROOT}/{pid}/{STAT_FILE}");
    let bytes = fs::read(&path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            SignalError::NoSuchProcess
        } else {
            SignalError::Os(error)
        }
    })?;
    parse_process_stat(&String::from_utf8_lossy(&bytes))
        .map(|sample| sample.id())
        .map_err(|error| SignalError::Os(io::Error::new(io::ErrorKind::InvalidData, error)))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::process::Command;

    use borbatop_core::StartTime;

    use super::*;

    fn identity_of(pid: u32) -> ProcessId {
        current_identity(Pid::new(pid)).unwrap()
    }

    #[test]
    fn pid_zero_is_rejected() {
        let id = ProcessId::new(Pid::new(0), StartTime::new(0));
        assert!(matches!(
            send_signal(id, Signal::Terminate),
            Err(SignalError::InvalidPid(_))
        ));
    }

    #[test]
    fn init_belongs_to_root() {
        let result = send_signal(identity_of(1), Signal::Hangup);
        assert!(matches!(result, Err(SignalError::NotPermitted)));
    }

    #[test]
    fn terminates_a_child_process_and_then_reports_it_gone() {
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        let id = identity_of(child.id());

        send_signal(id, Signal::Terminate).unwrap();
        let status = child.wait().unwrap();

        assert!(!status.success());
        assert!(matches!(
            send_signal(id, Signal::Terminate),
            Err(SignalError::NoSuchProcess)
        ));
    }

    #[test]
    fn a_reused_pid_is_never_signalled() {
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        let real = identity_of(child.id());
        let impostor = ProcessId::new(real.pid(), StartTime::new(real.start_time().get() + 1));

        let result = send_signal(impostor, Signal::Kill);

        assert!(matches!(result, Err(SignalError::NoSuchProcess)));
        assert!(
            child.try_wait().unwrap().is_none(),
            "the real process must survive"
        );
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn names_match_kill_l() {
        assert_eq!(Signal::Kill.name(), "SIGKILL");
        assert_eq!(Signal::Terminate.name(), "SIGTERM");
        assert_eq!(Signal::Hangup.name(), "SIGHUP");
    }
}
