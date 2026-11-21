//! Changing how the kernel schedules a process: niceness and CPU affinity.
//!
//! Linux keeps both per thread, so every function here walks `/proc/<pid>/task` and applies the
//! change to each thread. The process is identified by PID plus start time; the start time is
//! checked right before the change. Unlike signals there is no pidfd form of these syscalls, so
//! a PID reused in the microseconds after the check is a residual risk (the effect is a changed
//! priority or CPU mask, not a terminated process).

use std::error::Error;
use std::fmt;
use std::fs;
use std::io;

use borbatop_core::{Nice, Pid, ProcessId};
use rustix::io::Errno;
use rustix::process::{Pid as RawPid, setpriority_process};
use rustix::thread::{CpuSet, sched_setaffinity};

use crate::identity::{IdentityError, current_identity};

const PROC_ROOT: &str = "/proc";
const TASK_DIR: &str = "task";

/// Why a scheduling change was not applied.
#[derive(Debug)]
pub enum ScheduleError {
    /// The process belongs to another user, or raising priority needs privileges (`EPERM`/`EACCES`).
    NotPermitted,
    /// The process exited, or its PID now belongs to a different process.
    NoSuchProcess,
    /// The PID is zero or does not fit the kernel's PID type.
    InvalidPid(Pid),
    /// The CPU list is empty: a process cannot run nowhere.
    NoCpuSelected,
    /// A CPU index beyond what the kernel's mask can hold.
    CpuOutOfRange(usize),
    /// Any other OS error.
    Os(io::Error),
}

impl fmt::Display for ScheduleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPermitted => write!(formatter, "operation not permitted"),
            Self::NoSuchProcess => write!(formatter, "process already gone (ESRCH)"),
            Self::InvalidPid(pid) => write!(formatter, "invalid PID {pid}"),
            Self::NoCpuSelected => write!(formatter, "select at least one CPU"),
            Self::CpuOutOfRange(cpu) => write!(formatter, "CPU {cpu} is out of range"),
            Self::Os(error) => error.fmt(formatter),
        }
    }
}

impl Error for ScheduleError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Os(error) => Some(error),
            Self::NotPermitted
            | Self::NoSuchProcess
            | Self::InvalidPid(_)
            | Self::NoCpuSelected
            | Self::CpuOutOfRange(_) => None,
        }
    }
}

fn from_errno(errno: Errno) -> ScheduleError {
    if errno == Errno::PERM || errno == Errno::ACCESS {
        ScheduleError::NotPermitted
    } else if errno == Errno::SRCH {
        ScheduleError::NoSuchProcess
    } else {
        ScheduleError::Os(io::Error::from_raw_os_error(errno.raw_os_error()))
    }
}

/// Sets the niceness of every thread of the process.
///
/// # Arguments
///
/// * `id` - PID plus start time of the process
/// * `nice` - new niceness; lowering it below the current value needs privileges
///
/// # Errors
///
/// * [`ScheduleError::InvalidPid`] when the PID is zero or too large.
/// * [`ScheduleError::NoSuchProcess`] when the process exited or its PID was reused.
/// * [`ScheduleError::NotPermitted`] when the process is another user's or the kernel refuses.
/// * [`ScheduleError::Os`] for any other failure.
pub fn set_nice(id: ProcessId, nice: Nice) -> Result<(), ScheduleError> {
    let value = i32::from(nice.get());
    for_each_thread(id, |thread| {
        setpriority_process(Some(thread), value).map_err(from_errno)
    })
}

/// Restricts every thread of the process to the given CPUs.
///
/// # Arguments
///
/// * `id` - PID plus start time of the process
/// * `cpus` - indices of the CPUs the process may run on; must not be empty
///
/// # Errors
///
/// * [`ScheduleError::NoCpuSelected`] when `cpus` is empty.
/// * [`ScheduleError::CpuOutOfRange`] when an index exceeds the kernel's mask size.
/// * [`ScheduleError::InvalidPid`] when the PID is zero or too large.
/// * [`ScheduleError::NoSuchProcess`] when the process exited or its PID was reused.
/// * [`ScheduleError::NotPermitted`] when the process is another user's.
/// * [`ScheduleError::Os`] for any other failure (for example a CPU the process's cgroup
///   does not allow).
pub fn set_affinity(id: ProcessId, cpus: &[usize]) -> Result<(), ScheduleError> {
    if cpus.is_empty() {
        return Err(ScheduleError::NoCpuSelected);
    }
    let mut mask = CpuSet::new();
    for &cpu in cpus {
        if cpu >= CpuSet::MAX_CPU {
            return Err(ScheduleError::CpuOutOfRange(cpu));
        }
        mask.set(cpu);
    }
    for_each_thread(id, |thread| {
        sched_setaffinity(Some(thread), &mask).map_err(from_errno)
    })
}

/// Checks the identity of `id`, then runs `apply` on each of its threads.
///
/// A thread that exits while the loop runs is skipped; any other failure stops the walk.
fn for_each_thread(
    id: ProcessId,
    mut apply: impl FnMut(RawPid) -> Result<(), ScheduleError>,
) -> Result<(), ScheduleError> {
    let pid = id.pid();
    let process = raw_pid(pid.get()).ok_or(ScheduleError::InvalidPid(pid))?;

    let holder = current_identity(pid).map_err(|error| match error {
        IdentityError::Gone => ScheduleError::NoSuchProcess,
        IdentityError::Unreadable(source) => ScheduleError::Os(source),
    })?;
    if holder != id {
        return Err(ScheduleError::NoSuchProcess);
    }

    let mut applied = false;
    for thread in thread_ids(pid)? {
        let thread = raw_pid(thread).unwrap_or(process);
        match apply(thread) {
            Ok(()) => applied = true,
            Err(ScheduleError::NoSuchProcess) => {}
            Err(error) => return Err(error),
        }
    }
    if applied {
        Ok(())
    } else {
        Err(ScheduleError::NoSuchProcess)
    }
}

fn raw_pid(raw: u32) -> Option<RawPid> {
    i32::try_from(raw).ok().and_then(RawPid::from_raw)
}

fn thread_ids(pid: Pid) -> Result<Vec<u32>, ScheduleError> {
    let directory = format!("{PROC_ROOT}/{pid}/{TASK_DIR}");
    let entries = fs::read_dir(&directory).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            ScheduleError::NoSuchProcess
        } else {
            ScheduleError::Os(error)
        }
    })?;
    Ok(entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .collect())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::process::{Child, Command};

    use borbatop_core::StartTime;

    use super::*;

    fn spawn_sleeper() -> (Child, ProcessId) {
        let child = Command::new("sleep").arg("30").spawn().unwrap();
        let id = current_identity(Pid::new(child.id())).unwrap();
        (child, id)
    }

    fn stop(mut child: Child) {
        child.kill().unwrap();
        child.wait().unwrap();
    }

    fn status_field(pid: u32, name: &str) -> String {
        let text = fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
        text.lines()
            .find_map(|line| line.strip_prefix(name))
            .unwrap()
            .trim()
            .to_owned()
    }

    #[test]
    fn lowering_priority_is_allowed_and_visible_in_proc() {
        let (child, id) = spawn_sleeper();

        set_nice(id, Nice::clamped(7)).unwrap();

        let stat = fs::read_to_string(format!("/proc/{}/stat", child.id())).unwrap();
        assert_eq!(
            borbatop_core::parse_process_stat(&stat)
                .unwrap()
                .nice()
                .get(),
            7
        );
        stop(child);
    }

    #[test]
    fn affinity_is_applied_to_the_process() {
        let (child, id) = spawn_sleeper();

        let result = set_affinity(id, &[0]);

        // A restricted cpuset (some containers) may refuse CPU 0; anything else must work.
        if let Err(error) = result {
            assert!(matches!(error, ScheduleError::Os(_)), "{error}");
        } else {
            assert_eq!(status_field(child.id(), "Cpus_allowed_list:"), "0");
        }
        stop(child);
    }

    #[test]
    fn empty_and_out_of_range_cpu_lists_are_rejected() {
        let (child, id) = spawn_sleeper();
        assert!(matches!(
            set_affinity(id, &[]),
            Err(ScheduleError::NoCpuSelected)
        ));
        assert!(matches!(
            set_affinity(id, &[CpuSet::MAX_CPU]),
            Err(ScheduleError::CpuOutOfRange(_))
        ));
        stop(child);
    }

    #[test]
    fn a_reused_pid_is_left_alone() {
        let (child, real) = spawn_sleeper();
        let impostor = ProcessId::new(real.pid(), StartTime::new(real.start_time().get() + 1));

        assert!(matches!(
            set_nice(impostor, Nice::clamped(10)),
            Err(ScheduleError::NoSuchProcess)
        ));

        let stat = fs::read_to_string(format!("/proc/{}/stat", child.id())).unwrap();
        assert_eq!(
            borbatop_core::parse_process_stat(&stat)
                .unwrap()
                .nice()
                .get(),
            0
        );
        stop(child);
    }

    #[test]
    fn pid_zero_and_init_are_refused() {
        let zero = ProcessId::new(Pid::new(0), StartTime::new(0));
        assert!(matches!(
            set_nice(zero, Nice::clamped(1)),
            Err(ScheduleError::InvalidPid(_))
        ));

        let init = current_identity(Pid::new(1)).unwrap();
        assert!(matches!(
            set_nice(init, Nice::clamped(1)),
            Err(ScheduleError::NotPermitted)
        ));
    }
}
