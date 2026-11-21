//! Who owns a process and which systemd unit runs it.

use std::collections::HashMap;
use std::fmt;

const PASSWD_SEPARATOR: char = ':';
const PASSWD_UID_INDEX: usize = 2;
const COMMENT_PREFIX: char = '#';
const CGROUP_V2_PREFIX: &str = "0::";
const CGROUP_SEPARATOR: char = '/';

/// Numeric user identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Uid(u32);

impl Uid {
    /// Wraps a raw user identifier.
    ///
    /// # Arguments
    ///
    /// * `raw` - numeric UID as the kernel reports it
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw numeric UID.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for Uid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// User names indexed by UID, read from `/etc/passwd`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserTable {
    names: HashMap<Uid, Box<str>>,
    skipped: usize,
}

impl UserTable {
    /// Parses the contents of `/etc/passwd`.
    ///
    /// Blank lines and comments are ignored. A malformed line is counted in
    /// [`UserTable::skipped`] instead of being dropped silently. When two entries share
    /// a UID the first one wins, as in `getpwuid`.
    ///
    /// # Arguments
    ///
    /// * `text` - full contents of `/etc/passwd`
    ///
    /// # Examples
    ///
    /// ```
    /// use borbatop_core::{Uid, UserTable};
    ///
    /// let table = UserTable::parse("root:x:0:0:root:/root:/bin/bash\n");
    /// assert_eq!(table.name_of(Uid::new(0)), Some("root"));
    /// ```
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut names = HashMap::with_capacity(text.lines().count());
        let mut skipped = 0_usize;

        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with(COMMENT_PREFIX) {
                continue;
            }
            let mut fields = trimmed.split(PASSWD_SEPARATOR);
            let name = fields.next().filter(|name| !name.is_empty());
            let uid = fields
                .nth(PASSWD_UID_INDEX.saturating_sub(1))
                .and_then(|raw| raw.parse::<u32>().ok());
            match (name, uid) {
                (Some(name), Some(uid)) => {
                    names.entry(Uid(uid)).or_insert_with(|| name.into());
                }
                _ => skipped = skipped.saturating_add(1),
            }
        }

        Self { names, skipped }
    }

    /// Name of the user that owns `uid`, if the table knows it.
    ///
    /// # Arguments
    ///
    /// * `uid` - numeric user identifier
    #[must_use]
    pub fn name_of(&self, uid: Uid) -> Option<&str> {
        self.names.get(&uid).map(AsRef::as_ref)
    }

    /// Number of malformed lines that were not loaded.
    #[must_use]
    pub const fn skipped(&self) -> usize {
        self.skipped
    }
}

/// Name of the systemd unit (service or scope) that a process belongs to.
///
/// Reads the unified (v2) line of `/proc/[pid]/cgroup`: the unit is the last path component.
///
/// # Arguments
///
/// * `text` - full contents of `/proc/[pid]/cgroup`
///
/// # Returns
///
/// `None` when there is no v2 line or the process sits at the cgroup root.
///
/// # Examples
///
/// ```
/// use borbatop_core::parse_cgroup_unit;
///
/// let text = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-firefox-3310.scope\n";
/// assert_eq!(parse_cgroup_unit(text), Some("app-firefox-3310.scope"));
/// assert_eq!(parse_cgroup_unit("0::/init.scope\n"), Some("init.scope"));
/// assert_eq!(parse_cgroup_unit("0::/\n"), None);
/// ```
#[must_use]
pub fn parse_cgroup_unit(text: &str) -> Option<&str> {
    text.lines()
        .find_map(|line| line.strip_prefix(CGROUP_V2_PREFIX))
        .and_then(|path| {
            path.rsplit(CGROUP_SEPARATOR)
                .find(|component| !component.is_empty())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwd_first_entry_wins_and_bad_lines_are_counted() {
        let text = "# comment\n\nroot:x:0:0::/root:/bin/bash\nborba:x:1000:1000::/home/borba:/bin/bash\n\
                    alias:x:1000:1000::/home/x:/bin/sh\nbroken-line\n:x:5:5\nnouid:x:abc:1\n";
        let table = UserTable::parse(text);
        assert_eq!(table.name_of(Uid::new(0)), Some("root"));
        assert_eq!(table.name_of(Uid::new(1_000)), Some("borba"));
        assert_eq!(table.name_of(Uid::new(7)), None);
        assert_eq!(table.skipped(), 3);
    }

    #[test]
    fn cgroup_unit_handles_v1_noise_and_root() {
        let hybrid = "12:cpu:/user.slice\n0::/system.slice/sddm.service\n";
        assert_eq!(parse_cgroup_unit(hybrid), Some("sddm.service"));
        assert_eq!(parse_cgroup_unit("12:cpu:/user.slice\n"), None);
        assert_eq!(parse_cgroup_unit(""), None);
        assert_eq!(parse_cgroup_unit("0::/a/b/\n"), Some("b"));
    }
}
