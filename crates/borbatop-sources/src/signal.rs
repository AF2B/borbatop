//! Sending signals to processes through safe syscall wrappers.

use std::error::Error;
use std::fmt;
use std::io;

use borbatop_core::Pid;
use rustix::io::Errno;
use rustix::process::{Pid as RawPid, Signal as RawSignal, kill_process};

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
    /// The process no longer exists (`ESRCH`).
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
            Self::NoSuchProcess => write!(formatter, "no such process (ESRCH)"),
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

/// Sends `signal` to the process `pid`.
///
/// # Arguments
///
/// * `pid` - process to signal; zero is rejected so a process group is never hit by accident
/// * `signal` - signal to deliver
///
/// # Errors
///
/// * [`SignalError::InvalidPid`] when `pid` is zero or exceeds the kernel's PID range.
/// * [`SignalError::NotPermitted`] when the process belongs to another user.
/// * [`SignalError::NoSuchProcess`] when the process already exited.
/// * [`SignalError::Os`] for any other failure.
pub fn send_signal(pid: Pid, signal: Signal) -> Result<(), SignalError> {
    let raw_pid = i32::try_from(pid.get())
        .ok()
        .and_then(RawPid::from_raw)
        .ok_or(SignalError::InvalidPid(pid))?;

    kill_process(raw_pid, signal.raw()).map_err(|errno| {
        if errno == Errno::PERM {
            SignalError::NotPermitted
        } else if errno == Errno::SRCH {
            SignalError::NoSuchProcess
        } else {
            SignalError::Os(io::Error::from_raw_os_error(errno.raw_os_error()))
        }
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::process::Command;

    use super::*;

    #[test]
    fn pid_zero_is_rejected() {
        let result = send_signal(Pid::new(0), Signal::Terminate);
        assert!(matches!(result, Err(SignalError::InvalidPid(_))));
    }

    #[test]
    fn init_belongs_to_root() {
        let result = send_signal(Pid::new(1), Signal::Hangup);
        assert!(matches!(result, Err(SignalError::NotPermitted)));
    }

    #[test]
    fn terminates_a_child_process_and_then_reports_it_gone() {
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        let pid = Pid::new(child.id());

        send_signal(pid, Signal::Terminate).unwrap();
        let status = child.wait().unwrap();

        assert!(!status.success());
        let again = send_signal(pid, Signal::Terminate);
        assert!(matches!(again, Err(SignalError::NoSuchProcess)));
    }

    #[test]
    fn names_match_kill_l() {
        assert_eq!(Signal::Kill.name(), "SIGKILL");
        assert_eq!(Signal::Terminate.name(), "SIGTERM");
        assert_eq!(Signal::Hangup.name(), "SIGHUP");
    }
}
