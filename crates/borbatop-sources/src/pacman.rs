//! The Arch package manager's view of the machine: upgrades, orphans, kernel, leftovers.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use borbatop_core::{
    Bytes, Pid, Upgrade, deleted_libraries, is_pending_config, parse_name_list, parse_upgrades,
    reboot_recommended,
};

use crate::command::run;
use crate::error::{CommandError, Reading, SourceError};

const OSRELEASE: &str = "/proc/sys/kernel/osrelease";
const KERNEL_MODULES: &str = "/usr/lib/modules";
const CONFIG_ROOT: &str = "/etc";
const PACKAGE_CACHE: &str = "/var/cache/pacman/pkg";
const MAPS_FILE: &str = "maps";
/// `pacman -Qu` and `-Qdtq` exit with 1 when the list is empty.
const EMPTY_LIST_EXIT_CODE: i32 = 1;
const MAX_CONFIG_DEPTH: usize = 8;
/// AUR helpers that answer `-Qua` with the AUR packages that have a newer version.
const AUR_HELPERS: [&str; 2] = ["yay", "paru"];

/// Whether the running kernel matches what is installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KernelStatus {
    /// The running kernel's modules are on disk.
    UpToDate {
        /// Release of the running kernel.
        running: String,
    },
    /// The running kernel was replaced on disk: reboot to use the new one.
    RebootRecommended {
        /// Release of the running kernel.
        running: String,
    },
}

/// Configuration files left for review (`.pacnew`, `.pacsave`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingConfig {
    files: Vec<PathBuf>,
    unreadable_directories: usize,
}

impl PendingConfig {
    /// Files found, in directory order.
    #[must_use]
    pub fn files(&self) -> &[PathBuf] {
        &self.files
    }

    /// Directories or entries that could not be read (permissions).
    #[must_use]
    pub const fn unreadable_directories(&self) -> usize {
        self.unreadable_directories
    }
}

/// A running process that still maps a replaced or removed shared library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleProcess {
    pid: Pid,
    comm: String,
    library: String,
}

impl StaleProcess {
    /// The process to restart.
    #[must_use]
    pub const fn pid(&self) -> Pid {
        self.pid
    }

    /// Its command name.
    #[must_use]
    pub fn comm(&self) -> &str {
        &self.comm
    }

    /// The library whose file was replaced.
    #[must_use]
    pub fn library(&self) -> &str {
        &self.library
    }
}

/// Processes that map stale libraries, plus how many could not be inspected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleScan {
    findings: Vec<StaleProcess>,
    unreadable: usize,
}

impl StaleScan {
    /// One entry per process and library.
    #[must_use]
    pub fn findings(&self) -> &[StaleProcess] {
        &self.findings
    }

    /// Processes whose `maps` file is not readable by this user.
    #[must_use]
    pub const fn unreadable(&self) -> usize {
        self.unreadable
    }
}

/// Upgrades of packages built from the AUR, as an AUR helper reports them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AurUpgrades {
    helper: &'static str,
    upgrades: Vec<Upgrade>,
}

impl AurUpgrades {
    /// The helper that answered (`yay` or `paru`).
    #[must_use]
    pub const fn helper(&self) -> &'static str {
        self.helper
    }

    /// AUR packages with a newer version.
    #[must_use]
    pub fn upgrades(&self) -> &[Upgrade] {
        &self.upgrades
    }
}

/// Everything the Pacman tab shows, each part with its own success or failure.
#[derive(Debug)]
pub struct PacmanReport {
    upgrades: Reading<Vec<Upgrade>>,
    aur: Reading<Option<AurUpgrades>>,
    orphans: Reading<Vec<String>>,
    kernel: Reading<KernelStatus>,
    pending_config: Reading<PendingConfig>,
    cache_size: Reading<Bytes>,
    stale: StaleScan,
}

impl PacmanReport {
    /// Packages with a newer version in the already-synced database (no network access).
    #[must_use]
    pub const fn upgrades(&self) -> &Reading<Vec<Upgrade>> {
        &self.upgrades
    }

    /// AUR upgrades; `None` when no AUR helper is installed. Needs network access.
    #[must_use]
    pub const fn aur(&self) -> &Reading<Option<AurUpgrades>> {
        &self.aur
    }

    /// Packages installed as dependencies that nothing needs any more.
    #[must_use]
    pub const fn orphans(&self) -> &Reading<Vec<String>> {
        &self.orphans
    }

    /// Running kernel versus installed modules.
    #[must_use]
    pub const fn kernel(&self) -> &Reading<KernelStatus> {
        &self.kernel
    }

    /// `.pacnew` and `.pacsave` files under `/etc`.
    #[must_use]
    pub const fn pending_config(&self) -> &Reading<PendingConfig> {
        &self.pending_config
    }

    /// Size of the downloaded package cache.
    #[must_use]
    pub const fn cache_size(&self) -> &Reading<Bytes> {
        &self.cache_size
    }

    /// Processes using libraries that were replaced after they started.
    #[must_use]
    pub const fn stale(&self) -> &StaleScan {
        &self.stale
    }
}

/// Builds the Pacman report. Runs `pacman` twice and scans `/etc`, so call it on demand.
///
/// # Arguments
///
/// * `processes` - `(pid, command name)` of the running processes to inspect for stale libraries
#[must_use]
pub fn collect_pacman_report(processes: &[(Pid, String)]) -> PacmanReport {
    PacmanReport {
        upgrades: read_upgrades(),
        aur: read_aur(),
        orphans: read_orphans(),
        kernel: kernel_status(Path::new(OSRELEASE), Path::new(KERNEL_MODULES)),
        pending_config: scan_pending_config(Path::new(CONFIG_ROOT)),
        cache_size: cache_size(Path::new(PACKAGE_CACHE)),
        stale: scan_stale(Path::new("/proc"), processes),
    }
}

fn read_upgrades() -> Reading<Vec<Upgrade>> {
    let output = run("pacman", &["-Qu"], &[EMPTY_LIST_EXIT_CODE]).map_err(SourceError::Command)?;
    parse_upgrades(&output).map_err(|error| SourceError::parse("pacman -Qu", error))
}

/// Asks the first installed AUR helper for upgrades; no helper is not a failure.
fn read_aur() -> Reading<Option<AurUpgrades>> {
    for helper in AUR_HELPERS {
        match run(helper, &["-Qua"], &[EMPTY_LIST_EXIT_CODE]) {
            Ok(output) => {
                let upgrades = parse_upgrades(&output)
                    .map_err(|error| SourceError::parse("AUR helper -Qua", error))?;
                return Ok(Some(AurUpgrades { helper, upgrades }));
            }
            Err(CommandError::Spawn { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            }
            Err(error) => return Err(SourceError::Command(error)),
        }
    }
    Ok(None)
}

fn read_orphans() -> Reading<Vec<String>> {
    let output =
        run("pacman", &["-Qdtq"], &[EMPTY_LIST_EXIT_CODE]).map_err(SourceError::Command)?;
    Ok(parse_name_list(&output))
}

fn kernel_status(osrelease: &Path, modules: &Path) -> Reading<KernelStatus> {
    let running = fs::read_to_string(osrelease)
        .map_err(|error| SourceError::io(osrelease, error))?
        .trim()
        .to_owned();
    let installed: Vec<String> = fs::read_dir(modules)
        .map_err(|error| SourceError::io(modules, error))?
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .collect();
    let installed: Vec<&str> = installed.iter().map(String::as_str).collect();

    Ok(if reboot_recommended(&running, &installed) {
        KernelStatus::RebootRecommended { running }
    } else {
        KernelStatus::UpToDate { running }
    })
}

fn scan_pending_config(root: &Path) -> Reading<PendingConfig> {
    let mut files = Vec::new();
    let mut unreadable_directories = 0_usize;
    let mut stack = vec![(root.to_path_buf(), 0_usize)];
    let mut root_checked = false;

    while let Some((directory, depth)) = stack.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if !root_checked => return Err(SourceError::io(&directory, error)),
            Err(_) => {
                unreadable_directories = unreadable_directories.saturating_add(1);
                continue;
            }
        };
        root_checked = true;

        for entry in entries {
            let Ok((entry, kind)) =
                entry.and_then(|entry| entry.file_type().map(|kind| (entry, kind)))
            else {
                unreadable_directories = unreadable_directories.saturating_add(1);
                continue;
            };
            let name = entry.file_name();
            if kind.is_dir() && depth < MAX_CONFIG_DEPTH {
                stack.push((entry.path(), depth.saturating_add(1)));
            } else if kind.is_file() && name.to_str().is_some_and(is_pending_config) {
                files.push(entry.path());
            }
        }
    }

    files.sort();
    Ok(PendingConfig {
        files,
        unreadable_directories,
    })
}

fn cache_size(cache: &Path) -> Reading<Bytes> {
    let entries = match fs::read_dir(cache) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Bytes::new(0)),
        Err(error) => return Err(SourceError::io(cache, error)),
    };

    let mut total = 0_u64;
    for entry in entries.flatten() {
        if let Ok(metadata) = entry.metadata()
            && metadata.is_file()
        {
            total = total.saturating_add(metadata.len());
        }
    }
    Ok(Bytes::new(total))
}

fn scan_stale(proc_root: &Path, processes: &[(Pid, String)]) -> StaleScan {
    let mut findings = Vec::new();
    let mut unreadable = 0_usize;

    for (pid, comm) in processes {
        let maps = proc_root.join(pid.to_string()).join(MAPS_FILE);
        let Ok(bytes) = fs::read(&maps) else {
            unreadable = unreadable.saturating_add(1);
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        findings.extend(
            deleted_libraries(&text)
                .into_iter()
                .map(|library| StaleProcess {
                    pid: *pid,
                    comm: comm.clone(),
                    library: library.to_owned(),
                }),
        );
    }

    StaleScan {
        findings,
        unreadable,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("borbatop-pacman-{}-{name}", std::process::id()));
        match fs::remove_dir_all(&root) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => panic!("cannot clear fixture: {error}"),
        }
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn kernel_status_compares_release_with_module_directories() {
        let root = fixture("kernel");
        fs::create_dir_all(root.join("modules/7.2.12-arch1-1")).unwrap();
        fs::write(root.join("osrelease"), "7.2.11-arch1-1\n").unwrap();

        let stale = kernel_status(&root.join("osrelease"), &root.join("modules")).unwrap();
        assert_eq!(
            stale,
            KernelStatus::RebootRecommended {
                running: "7.2.11-arch1-1".into()
            }
        );

        fs::create_dir_all(root.join("modules/7.2.11-arch1-1")).unwrap();
        let current = kernel_status(&root.join("osrelease"), &root.join("modules")).unwrap();
        assert_eq!(
            current,
            KernelStatus::UpToDate {
                running: "7.2.11-arch1-1".into()
            }
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn pending_config_is_found_in_nested_directories() {
        let root = fixture("etc");
        fs::create_dir_all(root.join("pacman.d")).unwrap();
        fs::write(root.join("pacman.d/mirrorlist.pacnew"), "x").unwrap();
        fs::write(root.join("pacman.d/mirrorlist"), "x").unwrap();
        fs::write(root.join("fstab.pacsave"), "x").unwrap();

        let found = scan_pending_config(&root).unwrap();

        assert_eq!(found.files().len(), 2);
        assert_eq!(found.unreadable_directories(), 0);
        assert!(scan_pending_config(&root.join("absent")).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cache_size_sums_files_and_tolerates_a_missing_cache() {
        let root = fixture("cache");
        fs::write(root.join("a.pkg.tar.zst"), vec![0_u8; 100]).unwrap();
        fs::write(root.join("b.pkg.tar.zst"), vec![0_u8; 28]).unwrap();
        assert_eq!(cache_size(&root).unwrap(), Bytes::new(128));
        assert_eq!(cache_size(&root.join("absent")).unwrap(), Bytes::new(0));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn stale_scan_reports_findings_and_unreadable_processes() {
        let root = fixture("stale");
        fs::create_dir_all(root.join("7")).unwrap();
        fs::write(
            root.join("7/maps"),
            "a-b r-xp 0 fd:01 1   /usr/lib/libssl.so.3 (deleted)\n",
        )
        .unwrap();
        let processes = [
            (Pid::new(7), "nginx".to_owned()),
            (Pid::new(8), "gone".to_owned()),
        ];

        let scan = scan_stale(&root, &processes);

        assert_eq!(scan.findings().len(), 1);
        let finding = scan.findings().first().unwrap();
        assert_eq!(
            (finding.pid(), finding.comm(), finding.library()),
            (Pid::new(7), "nginx", "/usr/lib/libssl.so.3")
        );
        assert_eq!(scan.unreadable(), 1);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn real_report_reads_pacman_without_panicking() {
        let report = collect_pacman_report(&[]);
        // Kernel and upgrades depend on the host (containers have no modules or sync database).
        assert!(report.cache_size().is_ok());
        assert_eq!(report.stale().unreadable(), 0);
    }
}
