//! Facts about a process that never change during its life, resolved once and cached.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::Arc;

use borbatop_core::{ProcessId, ProcessSample, Uid, UserTable, parse_cgroup_unit};

use crate::owners::FileOwners;

/// Shown where a fact does not exist (a kernel thread has no unit, executable or package).
pub const NOT_AVAILABLE: &str = "—";
const CMDLINE_FILE: &str = "cmdline";
const CGROUP_FILE: &str = "cgroup";
const EXE_LINK: &str = "exe";
const MAX_CMDLINE_BYTES: u64 = 4_096;
const DELETED_SUFFIX: &str = " (deleted)";
const ARGUMENT_SEPARATOR: u8 = 0;

/// Owner, unit, package and command line of a process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessStatics {
    user: Box<str>,
    unit: Box<str>,
    package: Box<str>,
    command: Box<str>,
}

impl ProcessStatics {
    /// Name of the owning user, or the numeric UID when the name is unknown.
    #[must_use]
    pub fn user(&self) -> &str {
        &self.user
    }

    /// Systemd unit (service or scope), or [`NOT_AVAILABLE`].
    #[must_use]
    pub fn unit(&self) -> &str {
        &self.unit
    }

    /// Package that owns the executable, or [`NOT_AVAILABLE`].
    #[must_use]
    pub fn package(&self) -> &str {
        &self.package
    }

    /// Full command line, or `[name]` for kernel threads.
    #[must_use]
    pub fn command(&self) -> &str {
        &self.command
    }
}

/// Resolves [`ProcessStatics`] once per process incarnation and forgets finished processes.
#[derive(Debug)]
pub(crate) struct StaticsResolver {
    users: UserTable,
    owners: FileOwners,
    cache: HashMap<ProcessId, (Arc<ProcessStatics>, u64)>,
    generation: u64,
}

impl StaticsResolver {
    pub(crate) fn new(users: UserTable, owners: FileOwners) -> Self {
        Self {
            users,
            owners,
            cache: HashMap::new(),
            generation: 0,
        }
    }

    /// Starts a scan: entries not touched before [`StaticsResolver::end_scan`] are dropped.
    pub(crate) fn begin_scan(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    /// Drops cached facts of processes that were not seen in this scan.
    pub(crate) fn end_scan(&mut self) {
        let generation = self.generation;
        self.cache.retain(|_, (_, seen)| *seen == generation);
    }

    /// Facts of `sample`, reading procfs only on the first sight of this incarnation.
    pub(crate) fn resolve(
        &mut self,
        directory: &Path,
        sample: &ProcessSample,
    ) -> Arc<ProcessStatics> {
        let generation = self.generation;
        if let Some((statics, seen)) = self.cache.get_mut(&sample.id()) {
            *seen = generation;
            return Arc::clone(statics);
        }

        let statics = Arc::new(self.read(directory, sample));
        self.cache
            .insert(sample.id(), (Arc::clone(&statics), generation));
        statics
    }

    fn read(&self, directory: &Path, sample: &ProcessSample) -> ProcessStatics {
        let user = match fs::metadata(directory).map(|metadata| Uid::new(metadata.uid())) {
            Ok(uid) => self
                .users
                .name_of(uid)
                .map_or_else(|| uid.to_string(), str::to_owned),
            Err(_) => NOT_AVAILABLE.to_owned(),
        };
        let unit = fs::read_to_string(directory.join(CGROUP_FILE))
            .ok()
            .and_then(|text| parse_cgroup_unit(&text).map(str::to_owned))
            .unwrap_or_else(|| NOT_AVAILABLE.to_owned());
        let package = fs::read_link(directory.join(EXE_LINK))
            .ok()
            .and_then(|target| target.to_str().map(str::to_owned))
            .and_then(|target| {
                let path = target.strip_suffix(DELETED_SUFFIX).unwrap_or(&target);
                self.owners.package_of(path).map(str::to_owned)
            })
            .unwrap_or_else(|| NOT_AVAILABLE.to_owned());

        ProcessStatics {
            user: user.into(),
            unit: unit.into(),
            package: package.into(),
            command: read_command(directory, sample).into(),
        }
    }
}

fn read_command(directory: &Path, sample: &ProcessSample) -> String {
    let mut bytes = Vec::new();
    let read = File::open(directory.join(CMDLINE_FILE))
        .and_then(|file| file.take(MAX_CMDLINE_BYTES).read_to_end(&mut bytes));
    if read.is_err() || bytes.is_empty() {
        return format!("[{}]", sample.comm());
    }

    let arguments: Vec<String> = bytes
        .split(|byte| *byte == ARGUMENT_SEPARATOR)
        .filter(|argument| !argument.is_empty())
        .map(|argument| String::from_utf8_lossy(argument).into_owned())
        .collect();
    arguments.join(" ")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use borbatop_core::parse_process_stat;

    use super::*;

    const STAT: &str = "10 (init) S 0 1 1 0 -1 0 0 0 0 0 7 3 0 0 20 0 2 0 1 100 50 0";

    fn fixture(name: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("borbatop-statics-{}-{name}", std::process::id()));
        match fs::remove_dir_all(&root) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("cannot clear fixture: {error}"),
        }
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn reads_unit_and_command_once_and_caches_them() {
        let dir = fixture("cache");
        fs::write(dir.join(CGROUP_FILE), "0::/system.slice/sshd.service\n").unwrap();
        fs::write(dir.join(CMDLINE_FILE), b"/usr/bin/sshd\0-D\0-e\0").unwrap();
        let sample = parse_process_stat(STAT).unwrap();
        let mut resolver = StaticsResolver::new(UserTable::default(), FileOwners::default());

        resolver.begin_scan();
        let first = resolver.resolve(&dir, &sample);
        fs::remove_file(dir.join(CMDLINE_FILE)).unwrap();
        let second = resolver.resolve(&dir, &sample);

        assert_eq!(first.unit(), "sshd.service");
        assert_eq!(first.command(), "/usr/bin/sshd -D -e");
        assert_eq!(first.package(), NOT_AVAILABLE);
        assert!(Arc::ptr_eq(&first, &second));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn kernel_thread_falls_back_to_the_command_name() {
        let dir = fixture("kthread");
        fs::write(dir.join(CMDLINE_FILE), b"").unwrap();
        let sample = parse_process_stat(STAT).unwrap();
        let mut resolver = StaticsResolver::new(UserTable::default(), FileOwners::default());

        resolver.begin_scan();
        let statics = resolver.resolve(&dir, &sample);

        assert_eq!(statics.command(), "[init]");
        assert_eq!(statics.unit(), NOT_AVAILABLE);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unseen_processes_are_forgotten_after_a_scan() {
        let dir = fixture("forget");
        let sample = parse_process_stat(STAT).unwrap();
        let mut resolver = StaticsResolver::new(UserTable::default(), FileOwners::default());

        resolver.begin_scan();
        let first = resolver.resolve(&dir, &sample);
        resolver.end_scan();
        resolver.begin_scan();
        resolver.end_scan();
        resolver.begin_scan();
        let second = resolver.resolve(&dir, &sample);

        assert!(!Arc::ptr_eq(&first, &second));
        fs::remove_dir_all(&dir).unwrap();
    }
}
