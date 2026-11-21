//! The data the screen shows: a plain, already-derived snapshot of the machine.

use std::time::Duration;

use borbatop_core::{
    Bytes, BytesPerSecond, History, LoadAvg, MemInfo, MilliCelsius, Permille, Pid, Pressure,
    ProcessState, StartTime, ThreadCount, UnitList, Upgrade,
};

/// One row of the process table.
///
/// `Debug` hides the command line: arguments often carry passwords and personal data.
#[derive(Clone, PartialEq, Eq)]
pub struct ProcessRow {
    /// Process identifier.
    pub pid: Pid,
    /// When this incarnation of the PID started; with `pid` it identifies the process.
    pub start_time: StartTime,
    /// Parent process identifier.
    pub parent: Pid,
    /// Owner name, or numeric UID when unknown.
    pub user: String,
    /// Scheduler state.
    pub state: ProcessState,
    /// CPU usage over the last window, in permille of one core.
    pub cpu: Permille,
    /// Resident memory.
    pub memory: Bytes,
    /// Number of threads.
    pub threads: ThreadCount,
    /// Systemd unit that runs the process.
    pub unit: String,
    /// Package that owns the executable.
    pub package: String,
    /// Full command line.
    pub command: String,
}

impl std::fmt::Debug for ProcessRow {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProcessRow")
            .field("pid", &self.pid)
            .field("start_time", &self.start_time)
            .field("parent", &self.parent)
            .field("user", &self.user)
            .field("state", &self.state)
            .field("cpu", &self.cpu)
            .field("memory", &self.memory)
            .field("threads", &self.threads)
            .field("unit", &self.unit)
            .field("package", &self.package)
            .field("command", &"<redacted>")
            .finish()
    }
}

/// Pressure-stall information of the three resources.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PressureView {
    /// CPU pressure.
    pub cpu: Option<Pressure>,
    /// Memory pressure.
    pub memory: Option<Pressure>,
    /// I/O pressure.
    pub io: Option<Pressure>,
}

/// One network interface with its recent throughput.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetRow {
    /// Interface name.
    pub name: String,
    /// Current receive rate.
    pub received: Option<BytesPerSecond>,
    /// Current send rate.
    pub sent: Option<BytesPerSecond>,
    /// Receive rate history, in bytes per second.
    pub received_history: History<u64>,
    /// Send rate history, in bytes per second.
    pub sent_history: History<u64>,
}

/// One physical disk with its recent throughput.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskRow {
    /// Device name.
    pub name: String,
    /// Current read rate.
    pub read: Option<BytesPerSecond>,
    /// Current write rate.
    pub written: Option<BytesPerSecond>,
}

/// One mounted filesystem and its fill level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountRow {
    /// Mount point.
    pub mount_point: String,
    /// Filesystem type.
    pub fs_type: String,
    /// Filesystem size.
    pub total: Bytes,
    /// Space in use.
    pub used: Bytes,
}

/// Running kernel versus installed modules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelView {
    /// Release of the running kernel.
    pub running: String,
    /// Whether a reboot is recommended.
    pub reboot_recommended: bool,
}

/// A process that maps a replaced library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleView {
    /// Process to restart.
    pub pid: Pid,
    /// Its command name.
    pub comm: String,
    /// The replaced library.
    pub library: String,
}

/// Everything the Pacman tab shows; each part may have failed on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacmanView {
    /// Pending upgrades, or why they are unknown.
    pub upgrades: Result<Vec<Upgrade>, String>,
    /// Orphaned packages, or why they are unknown.
    pub orphans: Result<Vec<String>, String>,
    /// Kernel status, or why it is unknown.
    pub kernel: Result<KernelView, String>,
    /// Leftover `.pacnew` and `.pacsave` files, or why they are unknown.
    pub pending_config: Result<Vec<String>, String>,
    /// Size of the package cache, or why it is unknown.
    pub cache: Result<Bytes, String>,
    /// Processes using replaced libraries.
    pub stale: Vec<StaleView>,
    /// Processes that could not be inspected.
    pub stale_unreadable: usize,
}

/// Everything the screen shows for one sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model {
    /// Process table rows.
    pub processes: Vec<ProcessRow>,
    /// Processes that exited between listing and reading.
    pub vanished: usize,
    /// Processes that could not be read.
    pub failed: usize,
    /// Per-core usage.
    pub cores: Vec<Permille>,
    /// Machine-wide usage over time, in permille.
    pub cpu_history: History<u64>,
    /// Memory and swap.
    pub memory: Option<MemInfo>,
    /// Load averages.
    pub load: Option<LoadAvg>,
    /// Time since boot.
    pub uptime: Option<Duration>,
    /// CPU temperature.
    pub temperature: Option<MilliCelsius>,
    /// Pressure-stall information.
    pub pressure: PressureView,
    /// Release of the running kernel.
    pub kernel_release: String,
    /// Systemd units, or why they are unknown.
    pub services: Result<UnitList, String>,
    /// Network interfaces.
    pub network: Vec<NetRow>,
    /// Physical disks.
    pub disks: Vec<DiskRow>,
    /// Mounted filesystems.
    pub mounts: Vec<MountRow>,
}

impl Model {
    /// An empty model, shown before the first sample arrives.
    ///
    /// # Arguments
    ///
    /// * `history_len` - number of samples kept for sparklines
    #[must_use]
    pub fn empty(history_len: usize) -> Self {
        Self {
            processes: Vec::new(),
            vanished: 0,
            failed: 0,
            cores: Vec::new(),
            cpu_history: History::new(history_len),
            memory: None,
            load: None,
            uptime: None,
            temperature: None,
            pressure: PressureView::default(),
            kernel_release: String::new(),
            services: Err("waiting for the first sample".to_owned()),
            network: Vec::new(),
            disks: Vec::new(),
            mounts: Vec::new(),
        }
    }
}
