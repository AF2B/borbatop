//! Everything the kernel says about one process: files, sockets, limits and memory.

use std::fmt;
use std::fs;
use std::path::Path;

use borbatop_core::{
    Bytes, FdKind, Limit, ProcessId, SocketEntry, classify_fd_target, parse_limits, status_size,
};

use crate::connections::read_socket_tables;
use crate::error::{Reading, SourceError};
use crate::identity::{IdentityError, current_identity};

const PROC_ROOT: &str = "/proc";
const FD_DIR: &str = "fd";
const EXE_LINK: &str = "exe";
const CWD_LINK: &str = "cwd";
const LIMITS_FILE: &str = "limits";
const STATUS_FILE: &str = "status";
const RESIDENT_KEY: &str = "VmRSS";
const PEAK_KEY: &str = "VmHWM";
const SWAP_KEY: &str = "VmSwap";
const REDACTED: &str = "<redacted>";

/// One open file descriptor.
///
/// `Debug` hides what the descriptor points at: file names often carry personal data.
#[derive(Clone, PartialEq, Eq)]
pub struct OpenDescriptor {
    fd: u32,
    kind: FdKind,
}

impl fmt::Debug for OpenDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenDescriptor")
            .field("fd", &self.fd)
            .field("kind", &REDACTED)
            .finish()
    }
}

impl OpenDescriptor {
    /// Descriptor number.
    #[must_use]
    pub const fn fd(&self) -> u32 {
        self.fd
    }

    /// What it points at.
    #[must_use]
    pub const fn kind(&self) -> &FdKind {
        &self.kind
    }
}

/// What the kernel says about one process; each part may be unreadable on its own.
///
/// `Debug` hides the executable, directory and descriptors: paths often carry personal data.
pub struct ProcessDetail {
    exe: Reading<String>,
    cwd: Reading<String>,
    descriptors: Reading<Vec<OpenDescriptor>>,
    sockets: Vec<SocketEntry>,
    limits: Reading<Vec<Limit>>,
    resident: Option<Bytes>,
    peak_resident: Option<Bytes>,
    swapped: Option<Bytes>,
}

impl fmt::Debug for ProcessDetail {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessDetail")
            .field("exe", &REDACTED)
            .field("cwd", &REDACTED)
            .field("descriptors", &REDACTED)
            .field("sockets", &self.sockets.len())
            .field("resident", &self.resident)
            .finish_non_exhaustive()
    }
}

impl ProcessDetail {
    /// Path of the executable; ends in ` (deleted)` when the file was replaced.
    #[must_use]
    pub const fn exe(&self) -> &Reading<String> {
        &self.exe
    }

    /// Working directory.
    #[must_use]
    pub const fn cwd(&self) -> &Reading<String> {
        &self.cwd
    }

    /// Open descriptors, in descriptor order.
    #[must_use]
    pub const fn descriptors(&self) -> &Reading<Vec<OpenDescriptor>> {
        &self.descriptors
    }

    /// Sockets among the descriptors, with their addresses and states.
    #[must_use]
    pub fn sockets(&self) -> &[SocketEntry] {
        &self.sockets
    }

    /// Resource limits.
    #[must_use]
    pub const fn limits(&self) -> &Reading<Vec<Limit>> {
        &self.limits
    }

    /// Resident memory now.
    #[must_use]
    pub const fn resident(&self) -> Option<Bytes> {
        self.resident
    }

    /// Highest resident memory reached.
    #[must_use]
    pub const fn peak_resident(&self) -> Option<Bytes> {
        self.peak_resident
    }

    /// Memory pushed out to swap.
    #[must_use]
    pub const fn swapped(&self) -> Option<Bytes> {
        self.swapped
    }
}

/// Reads what the kernel says about one process.
///
/// The identity (PID plus start time) is checked before and after reading, so the answer never
/// mixes two processes that held the same PID.
///
/// # Arguments
///
/// * `id` - the process to inspect
///
/// # Errors
///
/// Returns [`SourceError::ProcessGone`] when the process exited or its PID was reused, and
/// [`SourceError::Io`] when its stat file cannot be read at all. Parts that are merely
/// unreadable (another user's descriptors) are reported inside the [`ProcessDetail`].
pub fn process_detail(id: ProcessId) -> Result<ProcessDetail, SourceError> {
    process_detail_in(Path::new(PROC_ROOT), id)
}

fn confirm(id: ProcessId) -> Result<(), SourceError> {
    match current_identity(id.pid()) {
        Ok(holder) if holder == id => Ok(()),
        Ok(_) | Err(IdentityError::Gone) => Err(SourceError::ProcessGone(id.pid())),
        Err(IdentityError::Unreadable(error)) => Err(SourceError::io(
            format!("{PROC_ROOT}/{}/stat", id.pid()),
            error,
        )),
    }
}

fn process_detail_in(proc_root: &Path, id: ProcessId) -> Result<ProcessDetail, SourceError> {
    confirm(id)?;
    let directory = proc_root.join(id.pid().to_string());

    let descriptors = read_descriptors(&directory);
    let sockets = match &descriptors {
        Ok(found) => sockets_of(proc_root, found),
        Err(_) => Vec::new(),
    };
    let status = fs::read_to_string(directory.join(STATUS_FILE)).unwrap_or_default();
    let detail = ProcessDetail {
        exe: read_link_text(&directory.join(EXE_LINK)),
        cwd: read_link_text(&directory.join(CWD_LINK)),
        descriptors,
        sockets,
        limits: fs::read_to_string(directory.join(LIMITS_FILE))
            .map(|text| parse_limits(&text))
            .map_err(|error| SourceError::io(directory.join(LIMITS_FILE), error)),
        resident: status_size(&status, RESIDENT_KEY),
        peak_resident: status_size(&status, PEAK_KEY),
        swapped: status_size(&status, SWAP_KEY),
    };

    confirm(id)?;
    Ok(detail)
}

fn read_link_text(path: &Path) -> Reading<String> {
    fs::read_link(path)
        .map(|target| target.to_string_lossy().into_owned())
        .map_err(|error| SourceError::io(path, error))
}

fn read_descriptors(directory: &Path) -> Reading<Vec<OpenDescriptor>> {
    let path = directory.join(FD_DIR);
    let listing = fs::read_dir(&path).map_err(|error| SourceError::io(&path, error))?;

    let mut found: Vec<OpenDescriptor> = listing
        .flatten()
        .filter_map(|entry| {
            let fd = entry.file_name().to_str()?.parse::<u32>().ok()?;
            let target = fs::read_link(entry.path()).ok()?;
            Some(OpenDescriptor {
                fd,
                kind: classify_fd_target(&target.to_string_lossy()),
            })
        })
        .collect();
    found.sort_by_key(OpenDescriptor::fd);
    Ok(found)
}

fn sockets_of(proc_root: &Path, descriptors: &[OpenDescriptor]) -> Vec<SocketEntry> {
    let inodes: Vec<u64> = descriptors
        .iter()
        .filter_map(|descriptor| match descriptor.kind() {
            FdKind::Socket(inode) => Some(*inode),
            FdKind::Pipe | FdKind::Anonymous(_) | FdKind::Path(_) | FdKind::Other(_) => None,
        })
        .collect();
    if inodes.is_empty() {
        return Vec::new();
    }
    // Sockets without a table entry (unix, netlink) are simply not listed here.
    let Ok((entries, _missing)) = read_socket_tables(proc_root) else {
        return Vec::new();
    };
    entries
        .into_iter()
        .filter(|entry| inodes.contains(&entry.inode()))
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::net::TcpListener;

    use borbatop_core::{Pid, StartTime};

    use super::*;

    fn own_id() -> ProcessId {
        current_identity(Pid::new(std::process::id())).unwrap()
    }

    #[test]
    fn this_process_reports_its_executable_files_and_limits() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let detail = process_detail(own_id()).unwrap();

        assert!(detail.exe().as_ref().unwrap().contains("borbatop"));
        assert!(detail.cwd().as_ref().unwrap().starts_with('/'));
        let descriptors = detail.descriptors().as_ref().unwrap();
        assert!(
            descriptors
                .iter()
                .any(|d| matches!(d.kind(), FdKind::Socket(_)))
        );
        assert!(
            detail
                .sockets()
                .iter()
                .any(|socket| socket.local().port() == port),
            "the listener must be among the sockets"
        );
        assert!(
            detail
                .limits()
                .as_ref()
                .unwrap()
                .iter()
                .any(|l| l.name == "Max open files")
        );
        assert!(detail.resident().unwrap().get() > 0);
        assert!(detail.peak_resident().unwrap() >= detail.resident().unwrap());
        drop(listener);
    }

    #[test]
    fn debug_output_hides_paths() {
        let detail = process_detail(own_id()).unwrap();
        let cwd = detail.cwd().as_ref().unwrap().clone();

        let printed = format!("{detail:?}");

        assert!(!printed.contains(&cwd), "{printed}");
        assert!(printed.contains("<redacted>"));
        let descriptor = detail.descriptors().as_ref().unwrap().first().unwrap();
        assert!(format!("{descriptor:?}").contains("<redacted>"));
    }

    #[test]
    fn a_reused_pid_is_refused() {
        let real = own_id();
        let impostor = ProcessId::new(real.pid(), StartTime::new(real.start_time().get() + 1));
        assert!(matches!(
            process_detail(impostor),
            Err(SourceError::ProcessGone(_))
        ));
    }

    #[test]
    fn another_users_process_reports_unreadable_parts_not_a_failure() {
        let init = current_identity(Pid::new(1)).unwrap();
        let detail = process_detail(init).unwrap();
        // pid 1 belongs to root: descriptors and exe are unreadable for a normal user.
        if !crate::identity::running_as_root() {
            assert!(detail.descriptors().is_err());
            assert_eq!(detail.sockets(), []);
        }
    }
}
