//! Parsers for `systemctl list-units` and for the Arch package manager's text output.

use std::error::Error;
use std::fmt;

const ARROW: &str = "->";
const FILES_HEADER: &str = "%FILES%";
const SECTION_MARK: char = '%';
const DIRECTORY_SUFFIX: char = '/';
const PACKAGE_DIR_PARTS: usize = 3;
const PACKAGE_SEPARATOR: char = '-';
const PACNEW_SUFFIXES: [&str; 2] = [".pacnew", ".pacsave"];
const DELETED_MARK: &str = " (deleted)";
const SHARED_OBJECT_MARK: &str = ".so";
const MAPS_PATH_FIELDS: usize = 6;
const UNIT_COLUMNS: usize = 4;

/// Activation state of a systemd unit (the ACTIVE column).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveState {
    /// Started and running or waiting.
    Active,
    /// Reloading its configuration.
    Reloading,
    /// Stopped.
    Inactive,
    /// Exited with an error or was killed.
    Failed,
    /// Starting up.
    Activating,
    /// Shutting down.
    Deactivating,
    /// Under maintenance.
    Maintenance,
    /// Refreshing its state.
    Refreshing,
}

impl ActiveState {
    fn from_text(text: &str) -> Option<Self> {
        match text {
            "active" => Some(Self::Active),
            "reloading" => Some(Self::Reloading),
            "inactive" => Some(Self::Inactive),
            "failed" => Some(Self::Failed),
            "activating" => Some(Self::Activating),
            "deactivating" => Some(Self::Deactivating),
            "maintenance" => Some(Self::Maintenance),
            "refreshing" => Some(Self::Refreshing),
            _ => None,
        }
    }

    /// Lower-case name, as `systemctl` prints it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Reloading => "reloading",
            Self::Inactive => "inactive",
            Self::Failed => "failed",
            Self::Activating => "activating",
            Self::Deactivating => "deactivating",
            Self::Maintenance => "maintenance",
            Self::Refreshing => "refreshing",
        }
    }
}

/// One line of `systemctl list-units`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitRow {
    name: String,
    active: ActiveState,
    sub: String,
    description: String,
}

impl UnitRow {
    /// Unit name (`sshd.service`).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Activation state.
    #[must_use]
    pub const fn active(&self) -> ActiveState {
        self.active
    }

    /// Low-level state (`running`, `waiting`, `dead`, `failed`, ...).
    #[must_use]
    pub fn sub(&self) -> &str {
        &self.sub
    }

    /// Human description.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }
}

/// Units parsed from `systemctl list-units`, plus the lines that could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitList {
    units: Vec<UnitRow>,
    skipped: usize,
}

impl UnitList {
    /// Units in the order `systemctl` printed them.
    #[must_use]
    pub fn units(&self) -> &[UnitRow] {
        &self.units
    }

    /// Lines with fewer than four columns or an unknown ACTIVE state.
    #[must_use]
    pub const fn skipped(&self) -> usize {
        self.skipped
    }
}

/// Parses the output of `systemctl list-units --all --plain --no-legend --no-pager`.
///
/// # Arguments
///
/// * `text` - standard output of the command
///
/// # Examples
///
/// ```
/// use borbatop_core::{parse_systemctl_units, ActiveState};
///
/// let text = "sshd.service loaded failed failed OpenSSH Daemon\n";
/// let list = parse_systemctl_units(text);
/// let unit = list.units().first().ok_or("no unit")?;
/// assert_eq!(unit.active(), ActiveState::Failed);
/// assert_eq!(unit.description(), "OpenSSH Daemon");
/// # Ok::<(), &str>(())
/// ```
#[must_use]
pub fn parse_systemctl_units(text: &str) -> UnitList {
    let mut units = Vec::with_capacity(text.lines().count());
    let mut skipped = 0_usize;

    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        match parse_unit_line(line) {
            Some(unit) => units.push(unit),
            None => skipped = skipped.saturating_add(1),
        }
    }

    UnitList { units, skipped }
}

fn parse_unit_line(line: &str) -> Option<UnitRow> {
    let mut rest = line.trim_start();
    let mut columns = [""; UNIT_COLUMNS];
    for column in &mut columns {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let (head, tail) = rest.split_at_checked(end)?;
        *column = head;
        rest = tail.trim_start();
    }
    let [name, _load, active, sub] = columns;

    Some(UnitRow {
        name: name.to_owned(),
        active: ActiveState::from_text(active)?,
        sub: sub.to_owned(),
        description: rest.trim_end().to_owned(),
    })
}

/// Why an Arch package manager output could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageLineError {
    line: String,
}

impl PackageLineError {
    /// The line that did not match `name old -> new`.
    #[must_use]
    pub fn line(&self) -> &str {
        &self.line
    }
}

impl fmt::Display for PackageLineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unrecognized package line: {:?}", self.line)
    }
}

impl Error for PackageLineError {}

/// A package with a newer version in the sync database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upgrade {
    name: String,
    from: String,
    to: String,
}

impl Upgrade {
    /// Package name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Installed version.
    #[must_use]
    pub fn from(&self) -> &str {
        &self.from
    }

    /// Available version.
    #[must_use]
    pub fn to(&self) -> &str {
        &self.to
    }
}

/// Parses the output of `pacman -Qu` (`name old -> new`, one per line).
///
/// # Arguments
///
/// * `text` - standard output of `pacman -Qu`
///
/// # Errors
///
/// Returns [`PackageLineError`] for the first line that does not match `name old -> new`.
///
/// # Examples
///
/// ```
/// use borbatop_core::parse_upgrades;
///
/// let upgrades = parse_upgrades("linux 7.2.11.arch1-1 -> 7.2.12.arch1-1\n")
///     .map_err(|error| error.to_string())?;
/// assert_eq!(upgrades.first().map(|upgrade| upgrade.to()), Some("7.2.12.arch1-1"));
/// # Ok::<(), String>(())
/// ```
pub fn parse_upgrades(text: &str) -> Result<Vec<Upgrade>, PackageLineError> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut tokens = line.split_ascii_whitespace();
            match (tokens.next(), tokens.next(), tokens.next(), tokens.next()) {
                (Some(name), Some(from), Some(ARROW), Some(to)) => Ok(Upgrade {
                    name: name.to_owned(),
                    from: from.to_owned(),
                    to: to.to_owned(),
                }),
                _ => Err(PackageLineError {
                    line: line.to_owned(),
                }),
            }
        })
        .collect()
}

/// Parses a plain list of names, one per line (`pacman -Qdtq`).
///
/// # Arguments
///
/// * `text` - standard output of the command
#[must_use]
pub fn parse_name_list(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Package name from a local database directory such as `aardvark-dns-2.1.0-3`.
///
/// # Arguments
///
/// * `directory` - directory name under `/var/lib/pacman/local`
///
/// # Returns
///
/// `None` when the directory is not `name-version-release`.
///
/// # Examples
///
/// ```
/// use borbatop_core::package_name_from_dir;
///
/// assert_eq!(package_name_from_dir("aardvark-dns-2.1.0-3"), Some("aardvark-dns"));
/// assert_eq!(package_name_from_dir("ALPM_DB_VERSION"), None);
/// ```
#[must_use]
pub fn package_name_from_dir(directory: &str) -> Option<&str> {
    let mut parts = directory.rsplitn(PACKAGE_DIR_PARTS, PACKAGE_SEPARATOR);
    let _release = parts.next()?;
    let _version = parts.next()?;
    parts.next().filter(|name| !name.is_empty())
}

/// File entries (not directories) of a package's `files` list, without the leading slash.
///
/// # Arguments
///
/// * `text` - contents of `/var/lib/pacman/local/<package>/files`
///
/// # Examples
///
/// ```
/// use borbatop_core::package_files;
///
/// let text = "%FILES%\nusr/\nusr/bin/\nusr/bin/bash\n\n%BACKUP%\netc/x\t1\n";
/// assert_eq!(package_files(text).collect::<Vec<_>>(), ["usr/bin/bash"]);
/// ```
pub fn package_files(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .skip_while(|line| *line != FILES_HEADER)
        .skip(1)
        .take_while(|line| !line.is_empty() && !line.starts_with(SECTION_MARK))
        .filter(|line| !line.ends_with(DIRECTORY_SUFFIX))
}

/// Whether a file name is a configuration file the package manager left for review.
///
/// # Arguments
///
/// * `file_name` - name of the file, without directory
///
/// # Examples
///
/// ```
/// use borbatop_core::is_pending_config;
///
/// assert!(is_pending_config("mirrorlist.pacnew"));
/// assert!(!is_pending_config("mirrorlist"));
/// ```
#[must_use]
pub fn is_pending_config(file_name: &str) -> bool {
    PACNEW_SUFFIXES
        .iter()
        .any(|suffix| file_name.ends_with(suffix))
}

/// Whether the running kernel's modules are still on disk.
///
/// After a kernel upgrade the old `/usr/lib/modules/<release>` directory disappears; that is
/// the classic sign that a reboot is due.
///
/// # Arguments
///
/// * `running_release` - contents of `/proc/sys/kernel/osrelease`
/// * `installed_releases` - directory names under `/usr/lib/modules`
///
/// # Examples
///
/// ```
/// use borbatop_core::reboot_recommended;
///
/// assert!(reboot_recommended("7.2.11-arch1-1", &["7.2.12-arch1-1"]));
/// assert!(!reboot_recommended("7.2.11-arch1-1", &["7.2.11-arch1-1"]));
/// ```
#[must_use]
pub fn reboot_recommended(running_release: &str, installed_releases: &[&str]) -> bool {
    let running = running_release.trim();
    !installed_releases.contains(&running)
}

/// Shared libraries a process still maps although their file was replaced or removed.
///
/// # Arguments
///
/// * `maps` - contents of `/proc/[pid]/maps`
///
/// # Returns
///
/// Each distinct path once, in order of first appearance.
///
/// # Examples
///
/// ```
/// use borbatop_core::deleted_libraries;
///
/// let maps = "7f00-7f01 r-xp 00000000 fd:01 42   /usr/lib/libssl.so.3 (deleted)\n\
///             7f02-7f03 r-xp 00000000 fd:01 43   /usr/lib/libc.so.6\n";
/// assert_eq!(deleted_libraries(maps), ["/usr/lib/libssl.so.3"]);
/// ```
#[must_use]
pub fn deleted_libraries(maps: &str) -> Vec<&str> {
    let mut libraries: Vec<&str> = Vec::new();

    for line in maps.lines() {
        let Some(path) = line
            .splitn(MAPS_PATH_FIELDS, char::is_whitespace)
            .nth(MAPS_PATH_FIELDS.saturating_sub(1))
            .map(str::trim_start)
            .and_then(|path| path.strip_suffix(DELETED_MARK))
        else {
            continue;
        };
        if path.contains(SHARED_OBJECT_MARK) && !libraries.contains(&path) {
            libraries.push(path);
        }
    }

    libraries
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn units_with_long_descriptions_and_bad_lines() {
        let text = "audit-rules.service loaded inactive dead Load Audit Rules\n\n\
                    cups.service loaded failed failed CUPS Scheduler\n\
                    odd.service loaded weird dead Nope\nshort.service loaded\n\
                    nodesc.service loaded active running\n";
        let list = parse_systemctl_units(text);
        assert_eq!(list.units().len(), 3);
        assert_eq!(list.skipped(), 2);
        let first = list.units().first().unwrap();
        assert_eq!(
            (first.name(), first.sub(), first.description()),
            ("audit-rules.service", "dead", "Load Audit Rules")
        );
        assert_eq!(list.units().get(1).unwrap().active().name(), "failed");
        assert_eq!(list.units().get(2).unwrap().description(), "");
    }

    #[test]
    fn upgrades_reject_a_malformed_line() {
        let ok = parse_upgrades("a 1 -> 2\nb 3 -> 4 [ignored]\n").unwrap();
        assert_eq!(ok.len(), 2);
        assert_eq!(ok.first().unwrap().from(), "1");
        let error = parse_upgrades("a 1 => 2\n").unwrap_err();
        assert_eq!(error.line(), "a 1 => 2");
        assert_eq!(parse_upgrades("").unwrap().len(), 0);
    }

    #[test]
    fn name_list_trims_and_drops_blanks() {
        assert_eq!(parse_name_list("lld\n \nopencv\n"), ["lld", "opencv"]);
    }

    #[test]
    fn package_dir_names_keep_dashes_in_the_name() {
        assert_eq!(package_name_from_dir("7zip-26.05-1"), Some("7zip"));
        assert_eq!(package_name_from_dir("a-b-c-1.0-2"), Some("a-b-c"));
        assert_eq!(package_name_from_dir("noversion"), None);
        assert_eq!(package_name_from_dir("-1.0-2"), None);
    }

    #[test]
    fn files_section_stops_at_the_next_section() {
        let text =
            "%FILES%\netc/\netc/bash.bashrc\nusr/bin/bash\n\n%BACKUP%\netc/bash.bashrc\thash\n";
        assert_eq!(
            package_files(text).collect::<Vec<_>>(),
            ["etc/bash.bashrc", "usr/bin/bash"]
        );
        assert_eq!(package_files("%NAME%\nbash\n").count(), 0);
    }

    #[test]
    fn pending_config_and_kernel_checks() {
        assert!(is_pending_config("pacman.conf.pacsave"));
        assert!(!is_pending_config("pacman.conf"));
        assert!(!reboot_recommended(
            "7.2.11-arch1-1\n",
            &["7.2.11-arch1-1", "7.2.12-arch1-1"]
        ));
        assert!(reboot_recommended("7.2.11-arch1-1", &[]));
    }

    #[test]
    fn deleted_libraries_ignore_data_files_and_duplicates() {
        let maps = "a-b r-xp 0 fd:01 1   /usr/lib/libssl.so.3 (deleted)\n\
                    c-d r--p 0 fd:01 1   /usr/lib/libssl.so.3 (deleted)\n\
                    e-f rw-s 0 00:01 9   /memfd:x (deleted)\n\
                    g-h r-xp 0 fd:01 2   /usr/lib/ok.so\n\
                    i-j r-xp 0 00:00 0\n";
        assert_eq!(deleted_libraries(maps), ["/usr/lib/libssl.so.3"]);
    }
}
