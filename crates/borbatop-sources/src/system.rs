//! Everything the main screen shows, read in one call: processes, memory, pressure, I/O.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use borbatop_core::{
    Bytes, DiskCounters, LoadAvg, MemInfo, MilliCelsius, Mount, NetCounters, PageSize, Pressure,
    parse_diskstats, parse_loadavg, parse_meminfo, parse_millicelsius, parse_mounts, parse_net_dev,
    parse_pressure, parse_uptime,
};

use crate::error::{Reading, SourceError};
use crate::proc::{ProcCollector, Snapshot};

const MEMINFO: &str = "/proc/meminfo";
const LOADAVG: &str = "/proc/loadavg";
const UPTIME: &str = "/proc/uptime";
const PRESSURE_CPU: &str = "/proc/pressure/cpu";
const PRESSURE_MEMORY: &str = "/proc/pressure/memory";
const PRESSURE_IO: &str = "/proc/pressure/io";
const NET_DEV: &str = "/proc/net/dev";
const DISKSTATS: &str = "/proc/diskstats";
const MOUNTS: &str = "/proc/mounts";
const SYS_BLOCK: &str = "/sys/block";
const HWMON: &str = "/sys/class/hwmon";
const HWMON_NAME: &str = "name";
const HWMON_TEMPERATURE: &str = "temp1_input";
/// Sensor drivers that report the CPU package temperature, best first.
const PREFERRED_SENSORS: [&str; 4] = ["coretemp", "k10temp", "zenpower", "acpitz"];
/// Block device name prefixes that are not physical disks.
const VIRTUAL_DISK_PREFIXES: [&str; 3] = ["loop", "ram", "zram"];

/// Space used on one mounted filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountUsage {
    mount: Mount,
    total: Bytes,
    used: Bytes,
}

impl MountUsage {
    /// The mounted filesystem.
    #[must_use]
    pub const fn mount(&self) -> &Mount {
        &self.mount
    }

    /// Size of the filesystem.
    #[must_use]
    pub const fn total(&self) -> Bytes {
        self.total
    }

    /// Space in use.
    #[must_use]
    pub const fn used(&self) -> Bytes {
        self.used
    }
}

/// Usage of every mounted block-device filesystem, plus how many could not be queried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountReport {
    usages: Vec<MountUsage>,
    unreadable: usize,
}

impl MountReport {
    /// One entry per distinct device, in mount order.
    #[must_use]
    pub fn usages(&self) -> &[MountUsage] {
        &self.usages
    }

    /// Mounts that `statvfs` refused or reported inconsistently.
    #[must_use]
    pub const fn unreadable(&self) -> usize {
        self.unreadable
    }
}

/// One consistent-enough read of the whole machine.
#[derive(Debug)]
pub struct SystemSnapshot {
    taken_at: Instant,
    processes: Snapshot,
    memory: Reading<MemInfo>,
    load: Reading<LoadAvg>,
    uptime: Reading<Duration>,
    cpu_pressure: Reading<Pressure>,
    memory_pressure: Reading<Pressure>,
    io_pressure: Reading<Pressure>,
    temperature: Reading<MilliCelsius>,
    network: Reading<Vec<NetCounters>>,
    disks: Reading<Vec<DiskCounters>>,
    mounts: Reading<MountReport>,
}

impl SystemSnapshot {
    /// When the read finished.
    #[must_use]
    pub const fn taken_at(&self) -> Instant {
        self.taken_at
    }

    /// Processes and CPU counters.
    #[must_use]
    pub const fn processes(&self) -> &Snapshot {
        &self.processes
    }

    /// Memory and swap.
    #[must_use]
    pub const fn memory(&self) -> &Reading<MemInfo> {
        &self.memory
    }

    /// Load averages.
    #[must_use]
    pub const fn load(&self) -> &Reading<LoadAvg> {
        &self.load
    }

    /// Time since boot.
    #[must_use]
    pub const fn uptime(&self) -> &Reading<Duration> {
        &self.uptime
    }

    /// CPU pressure (PSI).
    #[must_use]
    pub const fn cpu_pressure(&self) -> &Reading<Pressure> {
        &self.cpu_pressure
    }

    /// Memory pressure (PSI).
    #[must_use]
    pub const fn memory_pressure(&self) -> &Reading<Pressure> {
        &self.memory_pressure
    }

    /// I/O pressure (PSI).
    #[must_use]
    pub const fn io_pressure(&self) -> &Reading<Pressure> {
        &self.io_pressure
    }

    /// CPU package temperature.
    #[must_use]
    pub const fn temperature(&self) -> &Reading<MilliCelsius> {
        &self.temperature
    }

    /// Cumulative network counters per interface.
    #[must_use]
    pub const fn network(&self) -> &Reading<Vec<NetCounters>> {
        &self.network
    }

    /// Cumulative I/O counters per physical disk.
    #[must_use]
    pub const fn disks(&self) -> &Reading<Vec<DiskCounters>> {
        &self.disks
    }

    /// Space used per mounted filesystem.
    #[must_use]
    pub const fn mounts(&self) -> &Reading<MountReport> {
        &self.mounts
    }
}

/// Reads the whole machine; the process part is the only one that can fail the call.
#[derive(Debug)]
pub struct SystemCollector {
    processes: ProcCollector,
}

impl SystemCollector {
    /// Collector over the real `/proc`, `/sys` and `/etc`.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`ProcCollector::system`].
    pub fn system() -> Result<Self, SourceError> {
        Ok(Self {
            processes: ProcCollector::system()?,
        })
    }

    /// The memory page size of this machine.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::PageSize`] if the kernel reports zero.
    pub fn page_size() -> Result<PageSize, SourceError> {
        let raw = u64::try_from(rustix::param::page_size())
            .map_err(|_| SourceError::PageSize(borbatop_core::ZeroPageSize))?;
        PageSize::new(raw).map_err(SourceError::PageSize)
    }

    /// Reads processes, CPU, memory, pressure, temperature, network, disks and mounts.
    ///
    /// # Errors
    ///
    /// Only the process and CPU reading is mandatory; see [`ProcCollector::collect`]. Every
    /// other reading reports its own failure inside the snapshot.
    pub fn collect(&mut self) -> Result<SystemSnapshot, SourceError> {
        let processes = self.processes.collect()?;

        Ok(SystemSnapshot {
            taken_at: Instant::now(),
            processes,
            memory: read_parsed(MEMINFO, parse_meminfo),
            load: read_parsed(LOADAVG, parse_loadavg),
            uptime: read_parsed(UPTIME, parse_uptime),
            cpu_pressure: read_parsed(PRESSURE_CPU, parse_pressure),
            memory_pressure: read_parsed(PRESSURE_MEMORY, parse_pressure),
            io_pressure: read_parsed(PRESSURE_IO, parse_pressure),
            temperature: read_temperature(Path::new(HWMON)),
            network: read_parsed(NET_DEV, parse_net_dev),
            disks: read_disks(),
            mounts: read_mounts(),
        })
    }
}

fn read_text(path: &str) -> Reading<String> {
    fs::read(path)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .map_err(|error| SourceError::io(path, error))
}

fn read_parsed<T, E>(path: &str, parse: impl FnOnce(&str) -> Result<T, E>) -> Reading<T>
where
    E: std::error::Error + Send + Sync + 'static,
{
    let text = read_text(path)?;
    parse(&text).map_err(|error| SourceError::parse(path, error))
}

fn read_temperature(hwmon: &Path) -> Reading<MilliCelsius> {
    let entries = fs::read_dir(hwmon).map_err(|error| SourceError::io(hwmon, error))?;
    let mut best: Option<(usize, std::path::PathBuf)> = None;

    for entry in entries {
        let directory = entry.map_err(|error| SourceError::io(hwmon, error))?.path();
        let Ok(name) = fs::read_to_string(directory.join(HWMON_NAME)) else {
            continue;
        };
        let rank = PREFERRED_SENSORS
            .iter()
            .position(|sensor| *sensor == name.trim());
        if let Some(rank) = rank
            && best.as_ref().is_none_or(|(current, _)| rank < *current)
        {
            best = Some((rank, directory));
        }
    }

    let (_, directory) = best.ok_or_else(|| {
        SourceError::io(
            hwmon,
            io::Error::new(io::ErrorKind::NotFound, "no supported temperature sensor"),
        )
    })?;
    let file = directory.join(HWMON_TEMPERATURE);
    let text = fs::read_to_string(&file).map_err(|error| SourceError::io(&file, error))?;
    parse_millicelsius(&text).map_err(|error| SourceError::parse(file, error))
}

fn read_disks() -> Reading<Vec<DiskCounters>> {
    let physical = physical_disks(Path::new(SYS_BLOCK))?;
    let all = read_parsed(DISKSTATS, parse_diskstats)?;
    Ok(all
        .into_iter()
        .filter(|disk| physical.contains(disk.device()))
        .collect())
}

fn physical_disks(sys_block: &Path) -> Reading<HashSet<String>> {
    let entries = fs::read_dir(sys_block).map_err(|error| SourceError::io(sys_block, error))?;
    let mut names = HashSet::new();

    for entry in entries {
        let entry = entry.map_err(|error| SourceError::io(sys_block, error))?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if !VIRTUAL_DISK_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
        {
            names.insert(name);
        }
    }

    Ok(names)
}

fn read_mounts() -> Reading<MountReport> {
    let text = read_text(MOUNTS)?;
    let mut seen_devices = HashSet::new();
    let mut usages = Vec::new();
    let mut unreadable = 0_usize;

    for mount in parse_mounts(&text) {
        if !seen_devices.insert(mount.device().to_owned()) {
            continue;
        }
        match usage_of(mount) {
            Some(usage) => usages.push(usage),
            None => unreadable = unreadable.saturating_add(1),
        }
    }

    Ok(MountReport { usages, unreadable })
}

fn usage_of(mount: Mount) -> Option<MountUsage> {
    let stats = rustix::fs::statvfs(mount.mount_point()).ok()?;
    let total = stats.f_blocks.checked_mul(stats.f_frsize)?;
    let free = stats.f_bfree.checked_mul(stats.f_frsize)?;
    Some(MountUsage {
        mount,
        total: Bytes::new(total),
        used: Bytes::new(total.saturating_sub(free)),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn collects_the_whole_machine_without_hard_failure() {
        let mut collector = SystemCollector::system().unwrap();

        let snapshot = collector.collect().unwrap();

        assert_ne!(snapshot.processes().processes().len(), 0);
        assert!(snapshot.memory().as_ref().unwrap().total().get() > 0);
        assert!(snapshot.uptime().as_ref().unwrap().as_secs() > 0);
        assert!(snapshot.load().is_ok());
    }

    #[test]
    fn page_size_is_a_power_of_two() {
        let size = SystemCollector::page_size().unwrap();
        let pages = borbatop_core::parse_process_stat(
            "1 (a) S 0 1 1 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 1 1 1 0",
        )
        .unwrap()
        .resident_pages();
        let bytes = size.bytes_in(pages).unwrap().get();
        assert!(bytes.is_power_of_two());
    }

    #[test]
    fn mounted_root_has_usage_and_virtual_disks_are_excluded() {
        let report = read_mounts().unwrap();
        assert!(
            report
                .usages()
                .iter()
                .any(|usage| usage.mount().mount_point() == "/")
        );
        let physical = physical_disks(Path::new(SYS_BLOCK)).unwrap();
        assert!(!physical.iter().any(|name| name.starts_with("loop")));
    }

    #[test]
    fn missing_sensor_directory_is_reported() {
        let error = read_temperature(Path::new("/nonexistent/hwmon")).unwrap_err();
        assert!(matches!(error, SourceError::Io { .. }));
    }
}
