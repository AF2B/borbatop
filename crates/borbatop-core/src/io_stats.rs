//! Network, disk and mount parsers, and rate computation.

use std::error::Error;
use std::fmt;
use std::time::Duration;

use crate::bytes::Bytes;

const NET_HEADER_LINES: usize = 2;
const NET_RX_BYTES_INDEX: usize = 0;
const NET_TX_BYTES_INDEX: usize = 8;
const NET_TX_GAP: usize = NET_TX_BYTES_INDEX - NET_RX_BYTES_INDEX - 1;
const DISK_NAME_INDEX: usize = 2;
const DISK_SECTORS_READ_INDEX: usize = 5;
const DISK_SECTORS_WRITTEN_INDEX: usize = 9;
const DISK_MIN_FIELDS: usize = 10;
/// Sector size `/proc/diskstats` always uses, regardless of the device's real sector size.
const DISKSTATS_SECTOR_BYTES: u64 = 512;
const MILLIS_PER_SECOND: u128 = 1_000;
const MOUNT_FIELDS: usize = 3;
const MOUNT_ESCAPE: char = '\\';
const OCTAL_ESCAPE_DIGITS: usize = 3;
const OCTAL_RADIX: u32 = 8;
const BLOCK_DEVICE_PREFIX: &str = "/dev/";
const SQUASHFS: &str = "squashfs";

/// Why an I/O statistics file could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoStatsError {
    /// A line has fewer fields than the format requires.
    MissingField,
    /// A counter is not an unsigned integer.
    InvalidNumber,
}

impl fmt::Display for IoStatsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingField => write!(formatter, "i/o statistics line has too few fields"),
            Self::InvalidNumber => write!(formatter, "i/o statistics counter is not a number"),
        }
    }
}

impl Error for IoStatsError {}

/// Cumulative bytes received and sent by one network interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetCounters {
    interface: String,
    received: Bytes,
    sent: Bytes,
}

impl NetCounters {
    /// Interface name (`wlan0`, `eno1`).
    #[must_use]
    pub fn interface(&self) -> &str {
        &self.interface
    }

    /// Bytes received since boot.
    #[must_use]
    pub const fn received(&self) -> Bytes {
        self.received
    }

    /// Bytes sent since boot.
    #[must_use]
    pub const fn sent(&self) -> Bytes {
        self.sent
    }
}

/// Parses `/proc/net/dev`.
///
/// # Arguments
///
/// * `text` - full contents of `/proc/net/dev`, headers included
///
/// # Errors
///
/// * [`IoStatsError::MissingField`] when an interface line has fewer than nine counters.
/// * [`IoStatsError::InvalidNumber`] when a counter is not an unsigned integer.
///
/// # Examples
///
/// ```
/// use borbatop_core::parse_net_dev;
///
/// let text = "Inter-|   Receive\n face |bytes\n  eno1: 10 0 0 0 0 0 0 0 20 0 0 0 0 0 0 0\n";
/// let interfaces = parse_net_dev(text).map_err(|error| error.to_string())?;
/// assert_eq!(interfaces.len(), 1);
/// assert_eq!(interfaces.first().map(|net| net.sent().get()), Some(20));
/// # Ok::<(), String>(())
/// ```
pub fn parse_net_dev(text: &str) -> Result<Vec<NetCounters>, IoStatsError> {
    let mut interfaces = Vec::with_capacity(text.lines().count());

    for line in text.lines().skip(NET_HEADER_LINES) {
        let Some((name, counters)) = line.split_once(':') else {
            continue;
        };
        let mut fields = counters.split_ascii_whitespace();
        let received = nth_counter(&mut fields, NET_RX_BYTES_INDEX)?;
        let sent = nth_counter(&mut fields, NET_TX_GAP)?;
        interfaces.push(NetCounters {
            interface: name.trim().to_owned(),
            received: Bytes::new(received),
            sent: Bytes::new(sent),
        });
    }

    Ok(interfaces)
}

fn nth_counter<'a>(
    fields: &mut impl Iterator<Item = &'a str>,
    skip: usize,
) -> Result<u64, IoStatsError> {
    fields
        .nth(skip)
        .ok_or(IoStatsError::MissingField)?
        .parse()
        .map_err(|_| IoStatsError::InvalidNumber)
}

/// Cumulative bytes read from and written to one block device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskCounters {
    device: String,
    read: Bytes,
    written: Bytes,
}

impl DiskCounters {
    /// Device name (`nvme0n1`, `sda`).
    #[must_use]
    pub fn device(&self) -> &str {
        &self.device
    }

    /// Bytes read since boot.
    #[must_use]
    pub const fn read(&self) -> Bytes {
        self.read
    }

    /// Bytes written since boot.
    #[must_use]
    pub const fn written(&self) -> Bytes {
        self.written
    }
}

/// Parses `/proc/diskstats`.
///
/// # Arguments
///
/// * `text` - full contents of `/proc/diskstats`
///
/// # Errors
///
/// * [`IoStatsError::MissingField`] when a line has fewer than ten fields.
/// * [`IoStatsError::InvalidNumber`] when a sector counter is not an unsigned integer or the
///   byte count overflows.
///
/// # Examples
///
/// ```
/// use borbatop_core::parse_diskstats;
///
/// let text = " 259 0 nvme0n1 857457 68919 100 322221 1556943 2650 200 2329621 0 242510 2677595\n";
/// let disks = parse_diskstats(text).map_err(|error| error.to_string())?;
/// assert_eq!(disks.first().map(|disk| disk.read().get()), Some(100 * 512));
/// # Ok::<(), String>(())
/// ```
pub fn parse_diskstats(text: &str) -> Result<Vec<DiskCounters>, IoStatsError> {
    let mut disks = Vec::with_capacity(text.lines().count());

    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let fields: Vec<&str> = line.split_ascii_whitespace().collect();
        if fields.len() < DISK_MIN_FIELDS {
            return Err(IoStatsError::MissingField);
        }
        let device = fields
            .get(DISK_NAME_INDEX)
            .ok_or(IoStatsError::MissingField)?;
        disks.push(DiskCounters {
            device: (*device).to_owned(),
            read: sectors_to_bytes(fields.get(DISK_SECTORS_READ_INDEX))?,
            written: sectors_to_bytes(fields.get(DISK_SECTORS_WRITTEN_INDEX))?,
        });
    }

    Ok(disks)
}

fn sectors_to_bytes(field: Option<&&str>) -> Result<Bytes, IoStatsError> {
    let sectors: u64 = field
        .ok_or(IoStatsError::MissingField)?
        .parse()
        .map_err(|_| IoStatsError::InvalidNumber)?;
    sectors
        .checked_mul(DISKSTATS_SECTOR_BYTES)
        .map(Bytes::new)
        .ok_or(IoStatsError::InvalidNumber)
}

/// Throughput in bytes per second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BytesPerSecond(u64);

impl BytesPerSecond {
    /// The raw rate.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for BytesPerSecond {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.pad(&format!("{}/s", Bytes::new(self.0)))
    }
}

/// Throughput between two readings of a cumulative byte counter.
///
/// # Arguments
///
/// * `previous` - counter at the previous sample
/// * `current` - counter at this sample
/// * `elapsed` - wall-clock time between both samples
///
/// # Returns
///
/// `None` when no time elapsed, the counter went backwards (interface reset) or the rate
/// does not fit in 64 bits.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use borbatop_core::{bytes_per_second, Bytes};
///
/// let rate = bytes_per_second(Bytes::new(1_000), Bytes::new(3_000), Duration::from_secs(2));
/// assert_eq!(rate.map(|rate| rate.get()), Some(1_000));
/// ```
#[must_use]
pub fn bytes_per_second(
    previous: Bytes,
    current: Bytes,
    elapsed: Duration,
) -> Option<BytesPerSecond> {
    let delta = u128::from(current.get().checked_sub(previous.get())?);
    let millis = elapsed.as_millis();
    let rate = delta.checked_mul(MILLIS_PER_SECOND)?.checked_div(millis)?;
    u64::try_from(rate).ok().map(BytesPerSecond)
}

/// A mounted block-device filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    device: String,
    target: String,
    fs_type: String,
}

impl Mount {
    /// Device node (`/dev/nvme0n1p2`).
    #[must_use]
    pub fn device(&self) -> &str {
        &self.device
    }

    /// Where the filesystem is mounted.
    #[must_use]
    pub fn mount_point(&self) -> &str {
        &self.target
    }

    /// Filesystem type (`ext4`, `btrfs`).
    #[must_use]
    pub fn fs_type(&self) -> &str {
        &self.fs_type
    }
}

/// Parses `/proc/mounts`, keeping only filesystems backed by a real block device.
///
/// Pseudo filesystems (`proc`, `tmpfs`, ...) and read-only `squashfs` images are dropped.
/// Octal escapes such as `\040` in mount points are decoded.
///
/// # Arguments
///
/// * `text` - full contents of `/proc/mounts`
///
/// # Examples
///
/// ```
/// use borbatop_core::parse_mounts;
///
/// let text = "proc /proc proc rw 0 0\n/dev/sda1 /home ext4 rw,relatime 0 0\n";
/// let mounts = parse_mounts(text);
/// assert_eq!(mounts.len(), 1);
/// assert_eq!(mounts.first().map(|mount| mount.mount_point()), Some("/home"));
/// ```
#[must_use]
pub fn parse_mounts(text: &str) -> Vec<Mount> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_ascii_whitespace().take(MOUNT_FIELDS);
            let device = fields.next()?;
            let mount_point = fields.next()?;
            let fs_type = fields.next()?;
            let is_real = device.starts_with(BLOCK_DEVICE_PREFIX) && fs_type != SQUASHFS;
            is_real.then(|| Mount {
                device: unescape_mount(device),
                target: unescape_mount(mount_point),
                fs_type: fs_type.to_owned(),
            })
        })
        .collect()
}

fn unescape_mount(raw: &str) -> String {
    let mut decoded = String::with_capacity(raw.len());
    let mut chars = raw.chars();

    while let Some(character) = chars.next() {
        if character != MOUNT_ESCAPE {
            decoded.push(character);
            continue;
        }
        let digits: String = chars.clone().take(OCTAL_ESCAPE_DIGITS).collect();
        match u32::from_str_radix(&digits, OCTAL_RADIX)
            .ok()
            .and_then(char::from_u32)
        {
            Some(escaped) if digits.len() == OCTAL_ESCAPE_DIGITS => {
                decoded.push(escaped);
                chars.nth(OCTAL_ESCAPE_DIGITS.saturating_sub(1));
            }
            Some(_) | None => decoded.push(character),
        }
    }

    decoded
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const NET: &str = "Inter-|   Receive                 |  Transmit\n\
         face |bytes    packets errs drop fifo frame compressed multicast|bytes packets\n\
        lo: 1860215392 7927970 0 0 0 0 0 0 1860215393 7927970 0 0 0 0 0 0\n\
      eno1: 5 0 0 0 0 0 0 0 7 0 0 0 0 0 0 0\n";

    #[test]
    fn parses_receive_and_transmit_bytes() {
        let interfaces = parse_net_dev(NET).unwrap();
        assert_eq!(interfaces.len(), 2);
        assert_eq!(interfaces.first().unwrap().interface(), "lo");
        assert_eq!(
            interfaces.first().unwrap().received(),
            Bytes::new(1_860_215_392)
        );
        assert_eq!(
            interfaces.first().unwrap().sent(),
            Bytes::new(1_860_215_393)
        );
        assert_eq!(interfaces.get(1).unwrap().sent(), Bytes::new(7));
    }

    #[test]
    fn net_dev_rejects_short_and_garbled_lines() {
        let short = "h1\nh2\n eno1: 1 2 3\n";
        assert_eq!(parse_net_dev(short), Err(IoStatsError::MissingField));
        let garbled = "h1\nh2\n eno1: x 0 0 0 0 0 0 0 7 0\n";
        assert_eq!(parse_net_dev(garbled), Err(IoStatsError::InvalidNumber));
    }

    #[test]
    fn diskstats_converts_sectors_to_bytes() {
        let text =
            " 259 0 nvme0n1 857457 68919 183638962 322221 1556943 2650 86322028 2329621 0 1 2\n";
        let disks = parse_diskstats(text).unwrap();
        let disk = disks.first().unwrap();
        assert_eq!(disk.device(), "nvme0n1");
        assert_eq!(disk.read(), Bytes::new(183_638_962 * 512));
        assert_eq!(disk.written(), Bytes::new(86_322_028 * 512));
        assert_eq!(
            parse_diskstats(" 1 2 sda 1 2 3\n"),
            Err(IoStatsError::MissingField)
        );
        let garbled = " 1 2 sda 1 2 x 4 5 6 7\n";
        assert_eq!(parse_diskstats(garbled), Err(IoStatsError::InvalidNumber));
    }

    #[test]
    fn rate_needs_forward_time_and_counter() {
        let one_second = Duration::from_secs(1);
        assert_eq!(
            bytes_per_second(Bytes::new(0), Bytes::new(500), Duration::from_millis(500))
                .map(BytesPerSecond::get),
            Some(1_000)
        );
        assert_eq!(
            bytes_per_second(Bytes::new(5), Bytes::new(1), one_second),
            None
        );
        assert_eq!(
            bytes_per_second(Bytes::new(0), Bytes::new(1), Duration::ZERO),
            None
        );
        assert_eq!(BytesPerSecond(2_048).to_string(), "2.0K/s");
    }

    #[test]
    fn mounts_keep_block_devices_and_decode_escapes() {
        let text = "proc /proc proc rw 0 0\n/dev/loop3 /snap/x squashfs ro 0 0\n\
                    /dev/sda1 /mnt/my\\040disk ext4 rw 0 0\n/dev/sdb1 /odd\\04 ext4 rw 0 0\n\
                    tmpfs /run tmpfs rw 0 0\nshort\n";
        let mounts = parse_mounts(text);
        assert_eq!(mounts.len(), 2);
        assert_eq!(mounts.first().unwrap().mount_point(), "/mnt/my disk");
        assert_eq!(mounts.first().unwrap().fs_type(), "ext4");
        assert_eq!(mounts.get(1).unwrap().mount_point(), "/odd\\04");
    }
}
