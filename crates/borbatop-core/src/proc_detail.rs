//! What a process holds open and how it is limited: parsers for `/proc/[pid]/fd` link targets,
//! `/proc/[pid]/limits` and `/proc/[pid]/status`.

use crate::bytes::Bytes;
use crate::sockets::parse_socket_link;

const LIMIT_NAME_WIDTH: usize = 25;
const PIPE_PREFIX: &str = "pipe:[";
const ANON_PREFIX: &str = "anon_inode:";
const PATH_ROOT: char = '/';
const STATUS_SEPARATOR: char = ':';

/// What a file descriptor points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FdKind {
    /// A socket, with its inode (see [`crate::parse_socket_table`]).
    Socket(u64),
    /// One end of a pipe.
    Pipe,
    /// A kernel object without a path (`eventpoll`, `signalfd`, ...), with its type.
    Anonymous(String),
    /// A file, directory or device, with its path.
    Path(String),
    /// Anything else (for example `net:[4026531840]`).
    Other(String),
}

/// Classifies the target of a `/proc/[pid]/fd/N` link.
///
/// # Arguments
///
/// * `target` - what `readlink` returned
///
/// # Examples
///
/// ```
/// use borbatop_core::{FdKind, classify_fd_target};
///
/// assert_eq!(classify_fd_target("socket:[42]"), FdKind::Socket(42));
/// assert_eq!(classify_fd_target("/var/log/syslog"), FdKind::Path("/var/log/syslog".to_owned()));
/// ```
#[must_use]
pub fn classify_fd_target(target: &str) -> FdKind {
    if let Some(inode) = parse_socket_link(target) {
        return FdKind::Socket(inode);
    }
    if target.starts_with(PIPE_PREFIX) {
        return FdKind::Pipe;
    }
    if let Some(kind) = target.strip_prefix(ANON_PREFIX) {
        let kind = kind.trim_start_matches('[').trim_end_matches(']');
        return FdKind::Anonymous(kind.to_owned());
    }
    if target.starts_with(PATH_ROOT) {
        return FdKind::Path(target.to_owned());
    }
    FdKind::Other(target.to_owned())
}

/// One resource limit of a process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limit {
    /// Name, for example `Max open files`.
    pub name: String,
    /// Soft limit, or `unlimited`.
    pub soft: String,
    /// Hard limit, or `unlimited`.
    pub hard: String,
    /// Unit, empty when the limit is a plain count.
    pub unit: String,
}

/// Parses `/proc/[pid]/limits`; the header and malformed lines are left out.
///
/// # Arguments
///
/// * `text` - full contents of the file
#[must_use]
pub fn parse_limits(text: &str) -> Vec<Limit> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let name = line.get(..LIMIT_NAME_WIDTH)?.trim();
            let mut values = line.get(LIMIT_NAME_WIDTH..)?.split_ascii_whitespace();
            Some(Limit {
                name: name.to_owned(),
                soft: values.next()?.to_owned(),
                hard: values.next()?.to_owned(),
                unit: values.next().unwrap_or_default().to_owned(),
            })
        })
        .filter(|limit| !limit.name.is_empty())
        .collect()
}

/// A size field of `/proc/[pid]/status` such as `VmRSS:   1234 kB`.
///
/// # Arguments
///
/// * `text` - full contents of the file
/// * `key` - field name without the colon, for example `VmRSS`
#[must_use]
pub fn status_size(text: &str, key: &str) -> Option<Bytes> {
    text.lines().find_map(|line| {
        let (name, value) = line.split_once(STATUS_SEPARATOR)?;
        if name != key {
            return None;
        }
        let kib = value.split_ascii_whitespace().next()?.parse::<u64>().ok()?;
        Bytes::from_kib(kib)
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_of_fd_target_is_told_apart() {
        assert_eq!(classify_fd_target("socket:[7]"), FdKind::Socket(7));
        assert_eq!(classify_fd_target("pipe:[8]"), FdKind::Pipe);
        assert_eq!(
            classify_fd_target("anon_inode:[eventpoll]"),
            FdKind::Anonymous("eventpoll".to_owned())
        );
        assert_eq!(
            classify_fd_target("anon_inode:inotify"),
            FdKind::Anonymous("inotify".to_owned())
        );
        assert_eq!(
            classify_fd_target("/dev/null"),
            FdKind::Path("/dev/null".to_owned())
        );
        assert_eq!(
            classify_fd_target("/tmp/gone (deleted)"),
            FdKind::Path("/tmp/gone (deleted)".to_owned())
        );
        assert_eq!(
            classify_fd_target("net:[4026531840]"),
            FdKind::Other("net:[4026531840]".to_owned())
        );
    }

    #[test]
    fn limits_split_name_soft_hard_and_unit() {
        let text = "Limit                     Soft Limit           Hard Limit           Units     \nMax cpu time              unlimited            unlimited            seconds   \nMax open files            1024                 524288               files     \nMax processes             62318                62318                processes \nMax nice priority         0                    0                    \n";
        let limits = parse_limits(text);
        assert_eq!(limits.len(), 4);
        let files = limits.get(1).unwrap();
        assert_eq!(files.name, "Max open files");
        assert_eq!(files.soft, "1024");
        assert_eq!(files.hard, "524288");
        assert_eq!(files.unit, "files");
        assert_eq!(limits.first().unwrap().soft, "unlimited");
        assert_eq!(limits.get(3).unwrap().unit, "");
    }

    #[test]
    fn short_or_empty_limit_text_is_ignored() {
        assert_eq!(parse_limits(""), []);
        assert_eq!(parse_limits("header\nshort\n"), []);
    }

    #[test]
    fn status_sizes_are_converted_from_kib() {
        let text = "Name:\tfoo\nVmPeak:\t  2048 kB\nVmRSS:\t  1024 kB\nVmSwap:\t     0 kB\n";
        assert_eq!(status_size(text, "VmRSS").unwrap().get(), 1_048_576);
        assert_eq!(status_size(text, "VmSwap").unwrap().get(), 0);
        assert_eq!(status_size(text, "VmData"), None);
        assert_eq!(status_size("VmRSS: lots kB", "VmRSS"), None);
    }
}
