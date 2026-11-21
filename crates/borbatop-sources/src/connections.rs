//! Open sockets and the processes that own them, from `/proc/net` and `/proc/[pid]/fd`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::Path;

use borbatop_core::{Pid, SocketEntry, SocketProtocol, parse_socket_link, parse_socket_table};

use crate::error::SourceError;

const PROC_ROOT: &str = "/proc";
const NET_DIR: &str = "net";
const FD_DIR: &str = "fd";
const COMM_FILE: &str = "comm";
const UNKNOWN_COMM: &str = "?";

/// A process that holds a socket open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketOwner {
    pid: Pid,
    comm: String,
}

impl SocketOwner {
    /// An owner, for callers that build reports from their own data.
    ///
    /// # Arguments
    ///
    /// * `pid` - the process
    /// * `comm` - its command name
    #[must_use]
    pub fn new(pid: Pid, comm: &str) -> Self {
        Self {
            pid,
            comm: comm.to_owned(),
        }
    }

    /// The process.
    #[must_use]
    pub const fn pid(&self) -> Pid {
        self.pid
    }

    /// Its command name.
    #[must_use]
    pub fn comm(&self) -> &str {
        &self.comm
    }
}

/// One socket and who holds it; `owners` is empty when the owner is not visible to this user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    entry: SocketEntry,
    owners: Vec<SocketOwner>,
}

impl Connection {
    /// A connection with the processes known to hold it.
    ///
    /// # Arguments
    ///
    /// * `entry` - the socket table line
    /// * `owners` - processes holding the socket; empty when unknown
    #[must_use]
    pub const fn new(entry: SocketEntry, owners: Vec<SocketOwner>) -> Self {
        Self { entry, owners }
    }

    /// The socket table line.
    #[must_use]
    pub const fn entry(&self) -> &SocketEntry {
        &self.entry
    }

    /// Processes holding the socket (a socket can be shared with child processes).
    #[must_use]
    pub fn owners(&self) -> &[SocketOwner] {
        &self.owners
    }
}

/// Every socket of the machine, with owners where this user can see them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionReport {
    connections: Vec<Connection>,
    unreadable_processes: usize,
    missing_tables: usize,
}

impl ConnectionReport {
    /// A report from sockets already read.
    ///
    /// # Arguments
    ///
    /// * `connections` - sockets with their owners
    /// * `unreadable_processes` - processes whose descriptors could not be read
    /// * `missing_tables` - socket tables that could not be read
    #[must_use]
    pub const fn new(
        connections: Vec<Connection>,
        unreadable_processes: usize,
        missing_tables: usize,
    ) -> Self {
        Self {
            connections,
            unreadable_processes,
            missing_tables,
        }
    }

    /// The sockets, in table order.
    #[must_use]
    pub fn connections(&self) -> &[Connection] {
        &self.connections
    }

    /// Processes whose descriptors this user cannot read: their sockets show no owner.
    #[must_use]
    pub const fn unreadable_processes(&self) -> usize {
        self.unreadable_processes
    }

    /// Socket tables that could not be read (for example IPv6 disabled).
    #[must_use]
    pub const fn missing_tables(&self) -> usize {
        self.missing_tables
    }
}

/// Reads every socket table that exists; the second value counts the ones that did not.
///
/// # Errors
///
/// Returns the first read error when no table at all could be read.
pub(crate) fn read_socket_tables(
    proc_root: &Path,
) -> Result<(Vec<SocketEntry>, usize), SourceError> {
    let mut entries = Vec::new();
    let mut missing = 0_usize;
    let mut first_error = None;

    for protocol in SocketProtocol::ALL {
        let path = proc_root.join(NET_DIR).join(protocol.file_name());
        match fs::read_to_string(&path) {
            Ok(text) => entries.extend_from_slice(parse_socket_table(&text, protocol).entries()),
            Err(error) => {
                missing = missing.saturating_add(1);
                first_error.get_or_insert_with(|| SourceError::io(&path, error));
            }
        }
    }
    match first_error {
        Some(error) if missing == SocketProtocol::ALL.len() => Err(error),
        Some(_) | None => Ok((entries, missing)),
    }
}

/// Builds the connection report. Walks every process's descriptors, so call it on demand.
///
/// # Errors
///
/// Returns [`SourceError::Io`] when no socket table under `/proc/net` can be read.
pub fn collect_connections() -> Result<ConnectionReport, SourceError> {
    collect_connections_in(Path::new(PROC_ROOT))
}

fn collect_connections_in(proc_root: &Path) -> Result<ConnectionReport, SourceError> {
    let (entries, missing_tables) = read_socket_tables(proc_root)?;
    let (holders, unreadable_processes) = socket_holders(proc_root);

    let mut names: HashMap<Pid, String> = HashMap::new();
    let connections = entries
        .into_iter()
        .map(|entry| {
            let owners: Vec<SocketOwner> =
                holders.get(&entry.inode()).map_or_else(Vec::new, |pids| {
                    pids.iter()
                        .map(|&pid| SocketOwner {
                            pid,
                            comm: names
                                .entry(pid)
                                .or_insert_with(|| read_comm(proc_root, pid))
                                .clone(),
                        })
                        .collect()
                });
            Connection { entry, owners }
        })
        .collect();

    Ok(ConnectionReport {
        connections,
        unreadable_processes,
        missing_tables,
    })
}

/// Which processes hold which socket inode, and how many processes could not be inspected.
fn socket_holders(proc_root: &Path) -> (HashMap<u64, Vec<Pid>>, usize) {
    let mut holders: HashMap<u64, Vec<Pid>> = HashMap::new();
    let mut unreadable = 0_usize;
    let Ok(listing) = fs::read_dir(proc_root) else {
        return (holders, unreadable);
    };

    for entry in listing.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
            .map(Pid::new)
        else {
            continue;
        };
        let descriptors = match fs::read_dir(entry.path().join(FD_DIR)) {
            Ok(descriptors) => descriptors,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => {
                unreadable = unreadable.saturating_add(1);
                continue;
            }
        };

        let mut seen: HashSet<u64> = HashSet::new();
        for descriptor in descriptors.flatten() {
            let inode = fs::read_link(descriptor.path())
                .ok()
                .and_then(|target| parse_socket_link(&target.to_string_lossy()));
            if let Some(inode) = inode
                && seen.insert(inode)
            {
                holders.entry(inode).or_default().push(pid);
            }
        }
    }
    (holders, unreadable)
}

fn read_comm(proc_root: &Path, pid: Pid) -> String {
    fs::read(proc_root.join(pid.to_string()).join(COMM_FILE)).map_or_else(
        |_| UNKNOWN_COMM.to_owned(),
        |bytes| String::from_utf8_lossy(&bytes).trim().to_owned(),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::net::TcpListener;

    use super::*;

    #[test]
    fn a_listener_of_this_process_is_found_with_its_owner() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let report = collect_connections().unwrap();

        let found = report
            .connections()
            .iter()
            .find(|connection| connection.entry().local().port() == port)
            .expect("the listener must appear in /proc/net/tcp");
        let me = Pid::new(std::process::id());
        assert!(
            found.owners().iter().any(|owner| owner.pid() == me),
            "owners: {:?}",
            found.owners()
        );
        assert_eq!(found.entry().state().name(), "LISTEN");
        drop(listener);
    }

    #[test]
    fn a_missing_proc_net_is_an_error() {
        let error = collect_connections_in(Path::new("/nonexistent-proc")).unwrap_err();
        assert!(matches!(error, SourceError::Io { .. }));
    }
}
