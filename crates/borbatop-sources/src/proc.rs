//! Collector over procfs: one call, one consistent-enough snapshot of every process.

use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use borbatop_core::{
    CpuSnapshot, Pid, ProcessSample, StatParseError, UserTable, parse_cpu_stat, parse_process_stat,
};

use crate::error::SourceError;
use crate::owners::{FileOwners, PACMAN_LOCAL_DB};
use crate::statics::{ProcessStatics, StaticsResolver};

const SYSTEM_PROC_ROOT: &str = "/proc";
const PASSWD_FILE: &str = "/etc/passwd";
const STAT_FILE: &str = "stat";
const INITIAL_PROCESS_CAPACITY: usize = 512;
const INITIAL_BUFFER_CAPACITY: usize = 8 * 1024;
const INITIAL_PATH_CAPACITY: usize = 64;
/// `errno` the kernel returns when a process exits while its `/proc/[pid]` file is open.
const ESRCH: i32 = 3;

/// Why one process could not be sampled; the rest of the snapshot is still valid.
#[derive(Debug)]
pub enum ProcessFailureCause {
    /// Reading `/proc/[pid]/stat` failed for a reason other than the process exiting.
    Io(io::Error),
    /// The file was read but its contents are malformed.
    Parse(StatParseError),
}

/// A process that was listed but could not be sampled.
#[derive(Debug)]
pub struct ProcessFailure {
    pid: Pid,
    cause: ProcessFailureCause,
}

impl ProcessFailure {
    /// Process that failed.
    #[must_use]
    pub const fn pid(&self) -> Pid {
        self.pid
    }

    /// Reason it failed.
    #[must_use]
    pub const fn cause(&self) -> &ProcessFailureCause {
        &self.cause
    }
}

/// A sampled process together with the facts that never change during its life.
#[derive(Debug, Clone)]
pub struct ProcessRecord {
    sample: ProcessSample,
    statics: Arc<ProcessStatics>,
}

impl ProcessRecord {
    /// What `/proc/[pid]/stat` said at this sample.
    #[must_use]
    pub const fn sample(&self) -> &ProcessSample {
        &self.sample
    }

    /// Owner, unit, package and command line.
    #[must_use]
    pub fn statics(&self) -> &ProcessStatics {
        &self.statics
    }
}

/// One read of the whole system.
#[derive(Debug)]
pub struct Snapshot {
    processes: Vec<ProcessRecord>,
    failures: Vec<ProcessFailure>,
    vanished: usize,
    cpu: CpuSnapshot,
}

impl Snapshot {
    /// Processes sampled successfully.
    #[must_use]
    pub fn processes(&self) -> &[ProcessRecord] {
        &self.processes
    }

    /// Processes that were listed but could not be sampled.
    #[must_use]
    pub fn failures(&self) -> &[ProcessFailure] {
        &self.failures
    }

    /// Number of processes that exited between being listed and being read; expected noise.
    #[must_use]
    pub const fn vanished(&self) -> usize {
        self.vanished
    }

    /// Machine-wide and per-core CPU counters.
    #[must_use]
    pub const fn cpu(&self) -> &CpuSnapshot {
        &self.cpu
    }
}

/// Result of walking the process directories, before it is joined with the CPU counters.
struct ProcessScan {
    processes: Vec<ProcessRecord>,
    failures: Vec<ProcessFailure>,
    vanished: usize,
}

/// Reads procfs, reusing the byte buffer, the path buffer and the previous process count
/// across calls.
#[derive(Debug)]
pub struct ProcCollector {
    root: PathBuf,
    buffer: Vec<u8>,
    path: PathBuf,
    process_hint: usize,
    statics: StaticsResolver,
}

impl ProcCollector {
    /// Collector over an arbitrary procfs-shaped directory, with no user names or packages
    /// (tests use a fixture tree).
    ///
    /// # Arguments
    ///
    /// * `root` - directory holding `stat` and one numeric directory per process
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self::with_lookups(root, UserTable::default(), FileOwners::default())
    }

    /// Collector with user names and package ownership lookups.
    ///
    /// # Arguments
    ///
    /// * `root` - directory holding `stat` and one numeric directory per process
    /// * `users` - names to show instead of numeric UIDs
    /// * `owners` - package that owns each executable
    #[must_use]
    pub fn with_lookups(root: impl Into<PathBuf>, users: UserTable, owners: FileOwners) -> Self {
        Self {
            root: root.into(),
            buffer: Vec::with_capacity(INITIAL_BUFFER_CAPACITY),
            path: PathBuf::with_capacity(INITIAL_PATH_CAPACITY),
            process_hint: INITIAL_PROCESS_CAPACITY,
            statics: StaticsResolver::new(users, owners),
        }
    }

    /// Collector over the real `/proc`, with `/etc/passwd` and the pacman database loaded.
    ///
    /// Loading the package index takes a noticeable fraction of a second; a machine without
    /// pacman simply gets no package names.
    ///
    /// # Errors
    ///
    /// * [`SourceError::Io`] when `/etc/passwd` or the pacman database exists but cannot be read.
    pub fn system() -> Result<Self, SourceError> {
        let passwd =
            fs::read_to_string(PASSWD_FILE).map_err(|error| SourceError::io(PASSWD_FILE, error))?;
        let owners = FileOwners::load(Path::new(PACMAN_LOCAL_DB))?;
        Ok(Self::with_lookups(
            SYSTEM_PROC_ROOT,
            UserTable::parse(&passwd),
            owners,
        ))
    }

    /// Reads `/proc/stat` and `/proc/[pid]/stat` of every process.
    ///
    /// A process that exits mid-scan is counted in [`Snapshot::vanished`]. Any other
    /// per-process problem is returned as data in [`Snapshot::failures`], never dropped.
    /// Files are decoded as lossy UTF-8: the kernel cuts the command name at 15 bytes, which
    /// can split a multibyte character, and such a process must still be listed.
    ///
    /// # Errors
    ///
    /// * [`SourceError::Io`] when `/proc/stat` or the process directory cannot be read.
    /// * [`SourceError::CpuStat`] when `/proc/stat` is malformed.
    ///
    /// # Examples
    ///
    /// ```
    /// use borbatop_sources::ProcCollector;
    ///
    /// let snapshot = ProcCollector::system()?.collect()?;
    /// assert!(!snapshot.processes().is_empty());
    /// # Ok::<(), borbatop_sources::SourceError>(())
    /// ```
    pub fn collect(&mut self) -> Result<Snapshot, SourceError> {
        let cpu = self.read_cpu()?;
        let scan = self.scan_processes()?;
        self.process_hint = scan.processes.len();

        Ok(Snapshot {
            processes: scan.processes,
            failures: scan.failures,
            vanished: scan.vanished,
            cpu,
        })
    }

    fn read_cpu(&mut self) -> Result<CpuSnapshot, SourceError> {
        let stat_path = self.root.join(STAT_FILE);
        read_into(&stat_path, &mut self.buffer)
            .map_err(|source| SourceError::io(stat_path, source))?;
        parse_cpu_stat(&String::from_utf8_lossy(&self.buffer)).map_err(SourceError::CpuStat)
    }

    fn scan_processes(&mut self) -> Result<ProcessScan, SourceError> {
        let entries = fs::read_dir(&self.root)
            .map_err(|source| SourceError::io(self.root.clone(), source))?;
        self.statics.begin_scan();
        let mut scan = ProcessScan {
            processes: Vec::with_capacity(self.process_hint),
            failures: Vec::new(),
            vanished: 0,
        };

        for entry in entries {
            let entry = entry.map_err(|source| SourceError::io(self.root.clone(), source))?;
            let name = entry.file_name();
            let Some(pid) = pid_of(&name) else {
                continue;
            };

            self.path.as_mut_os_string().clear();
            self.path.push(&self.root);
            self.path.push(&name);
            self.path.push(STAT_FILE);

            match sample_process(&self.path, &mut self.buffer) {
                Ok(sample) => {
                    let directory = self.path.parent().unwrap_or(&self.root);
                    let statics = self.statics.resolve(directory, &sample);
                    scan.processes.push(ProcessRecord { sample, statics });
                }
                Err(ProcessFailureCause::Io(error)) if is_vanished(&error) => {
                    scan.vanished = scan.vanished.saturating_add(1);
                }
                Err(cause) => scan.failures.push(ProcessFailure { pid, cause }),
            }
        }

        self.statics.end_scan();
        Ok(scan)
    }
}

fn pid_of(name: &OsStr) -> Option<Pid> {
    name.to_str()?.parse::<u32>().ok().map(Pid::new)
}

fn sample_process(path: &Path, buffer: &mut Vec<u8>) -> Result<ProcessSample, ProcessFailureCause> {
    read_into(path, buffer).map_err(ProcessFailureCause::Io)?;
    parse_process_stat(&String::from_utf8_lossy(buffer)).map_err(ProcessFailureCause::Parse)
}

fn read_into(path: &Path, buffer: &mut Vec<u8>) -> io::Result<()> {
    buffer.clear();
    File::open(path)?.read_to_end(buffer).map(|_| ())
}

fn is_vanished(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound || error.raw_os_error() == Some(ESRCH)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use borbatop_core::CpuStatError;

    use super::*;

    const CPU_FIXTURE: &str = "cpu  100 5 50 800 20 3 2 1 0 0\n\
                               cpu0 50 2 25 400 10 1 1 0 0 0\n\
                               cpu1 50 3 25 400 10 2 1 1 0 0\n\
                               intr 12345 1 2\n";
    const VALID_STAT: &str =
        "10 (init) S 0 1 1 0 -1 4194560 0 0 0 0 7 3 0 0 20 0 2 0 1 100 50 18446744073709551615";

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    /// Fixture procfs tree under the system temp dir, removed on drop.
    struct FixtureTree {
        root: PathBuf,
    }

    impl FixtureTree {
        fn new() -> Self {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let root =
                std::env::temp_dir().join(format!("borbatop-fixture-{}-{id}", std::process::id()));
            match fs::remove_dir_all(&root) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => panic!("cannot clear stale fixture {}: {error}", root.display()),
            }
            fs::create_dir_all(&root).unwrap();
            fs::write(root.join(STAT_FILE), CPU_FIXTURE).unwrap();
            Self { root }
        }

        fn process(&self, name: &str, stat: Option<&[u8]>) {
            let dir = self.root.join(name);
            fs::create_dir_all(&dir).unwrap();
            if let Some(contents) = stat {
                fs::write(dir.join(STAT_FILE), contents).unwrap();
            }
        }
    }

    impl Drop for FixtureTree {
        fn drop(&mut self) {
            if let Err(error) = fs::remove_dir_all(&self.root) {
                eprintln!(
                    "fixture cleanup failed for {}: {error}",
                    self.root.display()
                );
            }
        }
    }

    #[test]
    fn collects_valid_processes_and_cpu() {
        let tree = FixtureTree::new();
        tree.process("10", Some(VALID_STAT.as_bytes()));
        tree.process(
            "20",
            Some(VALID_STAT.replace("10 (init)", "20 (sh)").as_bytes()),
        );

        let snapshot = ProcCollector::new(&tree.root).collect().unwrap();

        let mut pids: Vec<u32> = snapshot
            .processes()
            .iter()
            .map(|p| p.sample().pid().get())
            .collect();
        pids.sort_unstable();
        assert_eq!(pids, [10, 20]);
        assert_eq!(snapshot.cpu().cores().len(), 2);
        assert_eq!(snapshot.vanished(), 0);
        assert!(snapshot.failures().is_empty());
    }

    #[test]
    fn non_numeric_entries_are_ignored() {
        let tree = FixtureTree::new();
        tree.process("10", Some(VALID_STAT.as_bytes()));
        tree.process("self", Some(b"garbage"));
        tree.process("thread-self", Some(b"garbage"));
        fs::write(tree.root.join("uptime"), "1.0 1.0").unwrap();

        let snapshot = ProcCollector::new(&tree.root).collect().unwrap();

        assert_eq!(snapshot.processes().len(), 1);
        assert!(snapshot.failures().is_empty());
    }

    #[test]
    fn process_without_stat_counts_as_vanished() {
        let tree = FixtureTree::new();
        tree.process("10", Some(VALID_STAT.as_bytes()));
        tree.process("11", None);

        let snapshot = ProcCollector::new(&tree.root).collect().unwrap();

        assert_eq!(snapshot.processes().len(), 1);
        assert_eq!(snapshot.vanished(), 1);
        assert!(snapshot.failures().is_empty());
    }

    #[test]
    fn malformed_process_is_reported_not_dropped() {
        let tree = FixtureTree::new();
        tree.process("10", Some(VALID_STAT.as_bytes()));
        tree.process("12", Some(b"12 (broken) S 0 1"));

        let snapshot = ProcCollector::new(&tree.root).collect().unwrap();

        assert_eq!(snapshot.processes().len(), 1);
        let [failure] = snapshot.failures() else {
            panic!("expected exactly one failure");
        };
        assert_eq!(failure.pid(), Pid::new(12));
        assert!(matches!(failure.cause(), ProcessFailureCause::Parse(_)));
    }

    #[test]
    fn command_name_cut_inside_a_multibyte_character_is_still_listed() {
        let tree = FixtureTree::new();
        // The kernel keeps 15 bytes of `comm`; here the 2-byte "ã" (0xC3 0xA3) lost its tail.
        let mut line = b"42 (a\xc3) S ".to_vec();
        line.extend_from_slice(b"0 1 1 0 -1 0 0 0 0 0 7 3 0 0 20 0 2 0 1 100 50\n");
        tree.process("42", Some(&line));

        let snapshot = ProcCollector::new(&tree.root).collect().unwrap();

        assert!(snapshot.failures().is_empty());
        let [record] = snapshot.processes() else {
            panic!("expected exactly one process");
        };
        let sample = record.sample();
        assert_eq!(sample.comm(), "a\u{FFFD}");
    }

    #[test]
    fn collector_remembers_the_process_count_across_calls() {
        let tree = FixtureTree::new();
        tree.process("10", Some(VALID_STAT.as_bytes()));
        let mut collector = ProcCollector::new(&tree.root);

        let first = collector.collect().unwrap().processes().len();
        let second = collector.collect().unwrap().processes().len();

        assert_eq!((first, second), (1, 1));
        assert_eq!(collector.process_hint, 1);
    }

    #[test]
    fn missing_root_is_a_hard_error() {
        let tree = FixtureTree::new();
        let missing = tree.root.join("nope");

        let error = ProcCollector::new(&missing).collect().unwrap_err();

        assert!(matches!(error, SourceError::Io { .. }));
    }

    #[test]
    fn malformed_cpu_stat_is_a_hard_error() {
        let tree = FixtureTree::new();
        fs::write(tree.root.join(STAT_FILE), "intr 1\n").unwrap();

        let error = ProcCollector::new(&tree.root).collect().unwrap_err();

        assert!(matches!(
            error,
            SourceError::CpuStat(CpuStatError::MissingAggregate)
        ));
    }

    #[test]
    fn real_proc_contains_this_process() {
        let mut collector = ProcCollector::system().unwrap();

        let snapshot = collector.collect().unwrap();

        let own = Pid::new(std::process::id());
        let found = snapshot
            .processes()
            .iter()
            .find(|record| record.sample().pid() == own);
        let sample = found.expect("own process must be listed").sample();
        assert!(sample.threads().get() >= 1);
        assert_ne!(sample.comm(), "");
        assert_ne!(snapshot.cpu().cores(), []);
    }
}
