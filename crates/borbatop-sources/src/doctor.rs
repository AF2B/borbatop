//! What the Update Doctor looks at: the news feed, when the system was last upgraded, free disk
//! space, and which snapshot tool is installed.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use borbatop_core::{Bytes, NewsFeed, last_full_upgrade, parse_news_feed};

use crate::command::run;
use crate::error::{CommandError, Reading, SourceError};

const NEWS_URL: &str = "https://archlinux.org/feeds/news/";
const NEWS_TIMEOUT_SECONDS: &str = "10";
const PACMAN_LOG: &str = "/var/log/pacman.log";
const PACKAGE_CACHE: &str = "/var/cache/pacman/pkg";
const ROOT: &str = "/";
const PACNEW_SUFFIX: &str = ".pacnew";
const CONFIG_ROOT: &str = "/etc/";
const EXECUTABLE_BITS: u32 = 0o111;
/// `diff` exits with 1 when the files differ, which is the interesting case.
const DIFF_DIFFERENT_EXIT_CODE: i32 = 1;

/// A tool that can snapshot the system before an upgrade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotTool {
    /// `snapper`.
    Snapper,
    /// `timeshift`.
    Timeshift,
}

impl SnapshotTool {
    /// Every tool, in order of preference.
    pub const ALL: [Self; 2] = [Self::Snapper, Self::Timeshift];

    /// The program name.
    #[must_use]
    pub const fn program(self) -> &'static str {
        match self {
            Self::Snapper => "snapper",
            Self::Timeshift => "timeshift",
        }
    }

    /// The tool with this program name.
    ///
    /// # Arguments
    ///
    /// * `name` - program name, for example `snapper`
    #[must_use]
    pub fn from_program(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| tool.program() == name)
    }
}

/// The first snapshot tool found in `path_variable` (the value of `PATH`).
///
/// # Arguments
///
/// * `path_variable` - colon-separated directories to search
#[must_use]
pub fn find_snapshot_tool(path_variable: &str) -> Option<SnapshotTool> {
    SnapshotTool::ALL
        .into_iter()
        .find(|tool| is_in_path(tool.program(), path_variable))
}

/// The snapshot tool installed on this machine, if any.
#[must_use]
pub fn installed_snapshot_tool() -> Option<SnapshotTool> {
    find_snapshot_tool(&std::env::var("PATH").unwrap_or_default())
}

fn is_in_path(program: &str, path_variable: &str) -> bool {
    path_variable
        .split(':')
        .filter(|directory| !directory.is_empty())
        .any(|directory| {
            fs::metadata(Path::new(directory).join(program)).is_ok_and(|meta| {
                meta.is_file() && meta.permissions().mode() & EXECUTABLE_BITS != 0
            })
        })
}

/// Fetches the Arch Linux news feed.
///
/// Uses `curl` with a 10-second limit, so a missing network answers quickly.
///
/// # Errors
///
/// Returns [`CommandError`] when `curl` is missing, cannot connect, or the server answers with an
/// error.
pub fn fetch_news() -> Result<NewsFeed, CommandError> {
    run(
        "curl",
        &[
            "--fail",
            "--silent",
            "--show-error",
            "--max-time",
            NEWS_TIMEOUT_SECONDS,
            NEWS_URL,
        ],
        &[],
    )
    .map(|text| parse_news_feed(&text))
}

/// When the last full system upgrade started, from `/var/log/pacman.log`.
///
/// # Errors
///
/// Returns [`SourceError::Io`] when the log cannot be read. `Ok(None)` means it has no full
/// upgrade in it (a fresh install, or a rotated log).
pub fn last_upgrade_time() -> Reading<Option<u64>> {
    let log = fs::read_to_string(PACMAN_LOG).map_err(|error| SourceError::io(PACMAN_LOG, error))?;
    Ok(last_full_upgrade(&log))
}

/// Free and total space of one filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Headroom {
    path: &'static str,
    free: Bytes,
    total: Bytes,
}

impl Headroom {
    /// A measurement, for callers that build reports from their own data.
    ///
    /// # Arguments
    ///
    /// * `path` - the path that was measured
    /// * `free` - space available to a normal user
    /// * `total` - size of the filesystem
    #[must_use]
    pub const fn new(path: &'static str, free: Bytes, total: Bytes) -> Self {
        Self { path, free, total }
    }

    /// The path that was measured.
    #[must_use]
    pub const fn path(&self) -> &'static str {
        self.path
    }

    /// Space available to a normal user.
    #[must_use]
    pub const fn free(&self) -> Bytes {
        self.free
    }

    /// Size of the filesystem.
    #[must_use]
    pub const fn total(&self) -> Bytes {
        self.total
    }
}

/// Free space where an upgrade writes: the root filesystem and the package cache.
///
/// A path that cannot be measured is reported with its own error.
#[must_use]
pub fn disk_headroom() -> Vec<Reading<Headroom>> {
    [ROOT, PACKAGE_CACHE].into_iter().map(measure).collect()
}

fn measure(path: &'static str) -> Reading<Headroom> {
    let stats = rustix::fs::statvfs(path).map_err(|errno| {
        SourceError::io(
            path,
            std::io::Error::from_raw_os_error(errno.raw_os_error()),
        )
    })?;
    let overflow = || {
        SourceError::io(
            path,
            std::io::Error::new(std::io::ErrorKind::InvalidData, "size overflows 64 bits"),
        )
    };
    Ok(Headroom {
        path,
        free: Bytes::new(
            stats
                .f_bavail
                .checked_mul(stats.f_frsize)
                .ok_or_else(overflow)?,
        ),
        total: Bytes::new(
            stats
                .f_blocks
                .checked_mul(stats.f_frsize)
                .ok_or_else(overflow)?,
        ),
    })
}

/// What changed between a configuration file and the `.pacnew` the package left beside it.
///
/// # Arguments
///
/// * `pacnew` - absolute path of a `.pacnew` file under `/etc`
///
/// # Errors
///
/// * [`CommandError::InvalidConfigFile`] when the path is not a `.pacnew` under `/etc` (no `..`
///   either), so nothing else can be read through this function.
/// * [`CommandError::Spawn`] / [`CommandError::Failed`] when `diff` cannot run.
pub fn pacnew_diff(pacnew: &str) -> Result<String, CommandError> {
    let original = pacnew_original(pacnew)
        .ok_or_else(|| CommandError::InvalidConfigFile(pacnew.to_owned()))?;
    run(
        "diff",
        &["--unified", "--", original, pacnew],
        &[DIFF_DIFFERENT_EXIT_CODE],
    )
}

/// The file a `.pacnew` belongs to, when `pacnew` is a safe path to read.
fn pacnew_original(pacnew: &str) -> Option<&str> {
    let original = pacnew.strip_suffix(PACNEW_SUFFIX)?;
    let is_safe = pacnew.starts_with(CONFIG_ROOT)
        && !original.is_empty()
        && !pacnew.split('/').any(|part| part == "..")
        && !pacnew.contains('\0');
    is_safe.then_some(original)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn fake_bin(tools: &[&str]) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "borbatop-doctor-{}-{}",
            std::process::id(),
            tools.join("-")
        ));
        fs::create_dir_all(&directory).unwrap();
        for tool in tools {
            let path = directory.join(tool);
            fs::write(&path, "#!/bin/sh\n").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        directory
    }

    #[test]
    fn the_first_installed_snapshot_tool_wins_in_order_of_preference() {
        let both = fake_bin(&["timeshift", "snapper"]);
        let only_timeshift = fake_bin(&["timeshift"]);
        let none = fake_bin(&["something-else"]);

        assert_eq!(
            find_snapshot_tool(both.to_str().unwrap()),
            Some(SnapshotTool::Snapper)
        );
        assert_eq!(
            find_snapshot_tool(only_timeshift.to_str().unwrap()),
            Some(SnapshotTool::Timeshift)
        );
        assert_eq!(find_snapshot_tool(none.to_str().unwrap()), None);
        assert_eq!(find_snapshot_tool(""), None);
        assert_eq!(
            find_snapshot_tool(&format!("/nonexistent:{}", only_timeshift.display())),
            Some(SnapshotTool::Timeshift)
        );
        for directory in [both, only_timeshift, none] {
            fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn a_file_that_is_not_executable_is_not_a_tool() {
        let directory = fake_bin(&[]);
        fs::write(directory.join("snapper"), "x").unwrap();
        assert_eq!(find_snapshot_tool(directory.to_str().unwrap()), None);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn tools_round_trip_through_their_program_name() {
        for tool in SnapshotTool::ALL {
            assert_eq!(SnapshotTool::from_program(tool.program()), Some(tool));
        }
        assert_eq!(SnapshotTool::from_program("rm"), None);
    }

    #[test]
    fn the_root_filesystem_can_be_measured() {
        let headroom = disk_headroom();
        let root = headroom.first().unwrap().as_ref().unwrap();
        assert_eq!(root.path(), "/");
        assert!(root.total().get() > 0);
        assert!(root.free() <= root.total());
    }

    #[test]
    fn only_pacnew_files_under_etc_can_be_diffed() {
        assert_eq!(
            pacnew_original("/etc/pacman.conf.pacnew"),
            Some("/etc/pacman.conf")
        );
        for bad in [
            "/etc/passwd",
            "/home/a/x.pacnew",
            "/etc/../root/x.pacnew",
            "/etc/x.pacsave",
            ".pacnew",
            "etc/x.pacnew",
            "/etc/x\0y.pacnew",
        ] {
            assert_eq!(pacnew_original(bad), None, "{bad:?}");
        }
        assert!(matches!(
            pacnew_diff("/etc/passwd"),
            Err(CommandError::InvalidConfigFile(_))
        ));
    }

    #[test]
    fn a_missing_pacman_log_is_an_error_not_a_panic() {
        // The host may or may not have the log; either answer must be a value.
        let _answer = last_upgrade_time();
    }
}
