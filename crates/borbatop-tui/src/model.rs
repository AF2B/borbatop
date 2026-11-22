//! The data the screen shows: a plain, already-derived snapshot of the machine.

use std::time::Duration;

use borbatop_core::{
    Bytes, BytesPerSecond, History, LoadAvg, MemInfo, MilliCelsius, Nice, Permille, Pid, Pressure,
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
    /// Scheduling niceness.
    pub nice: Nice,
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
            .field("nice", &self.nice)
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

/// Upgrades of AUR packages and the helper that found them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AurView {
    /// Helper that answered, for example `yay`.
    pub helper: String,
    /// AUR packages with a newer version, or why the helper could not tell.
    pub upgrades: Result<Vec<Upgrade>, String>,
}

/// How the list of pending repository upgrades was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoSource {
    /// `checkupdates`: a temporary copy of the database, so the list is fresh.
    Checkupdates,
    /// `pacman -Qu`: only what the last synchronization of the local database knows.
    LocalDatabase,
}

/// Everything the Pacman tab shows; each part may have failed on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacmanView {
    /// Pending upgrades, or why they are unknown.
    pub upgrades: Result<Vec<Upgrade>, String>,
    /// How the repository upgrades were found.
    pub repo_source: RepoSource,
    /// Program name of the snapshot tool installed on the machine (`snapper` or `timeshift`).
    pub snapshot_tool: Option<String>,
    /// AUR upgrades, one entry per installed AUR helper (none when there is no helper).
    pub aur: Vec<AurView>,
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

/// One container as the runtime lists it, with what its processes use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerView {
    /// Runtime that owns it (`podman` or `docker`).
    pub runtime: String,
    /// Identifier, as the runtime prints it.
    pub id: String,
    /// Name given to the container.
    pub name: String,
    /// Image it was created from.
    pub image: String,
    /// Status text, for example `Up 3 hours`.
    pub status: String,
    /// Whether it is running now.
    pub running: bool,
    /// CPU and memory of its processes, ready to show.
    pub usage: String,
}

/// Everything the Containers tab shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContainersView {
    /// Containers of every installed runtime, running or not.
    pub containers: Vec<ContainerView>,
    /// Runtimes that could not list their containers, with the reason.
    pub problems: Vec<String>,
    /// Whether at least one runtime answered.
    pub has_runtime: bool,
    /// Container scopes that run processes but that no runtime listed, ready to show.
    pub unlisted: Vec<String>,
}

/// How a line of a [`Report`] should look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportTone {
    /// A section title.
    Heading,
    /// Ordinary text.
    Normal,
    /// Secondary text.
    Dim,
    /// Something that is fine.
    Good,
    /// Something that needs attention.
    Bad,
}

/// One line of a [`Report`]: text already formatted for the screen, and how loud it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportLine {
    /// The text; control characters are replaced when drawn.
    pub text: String,
    /// How it should look.
    pub tone: ReportTone,
}

impl ReportLine {
    /// A line with the given tone.
    ///
    /// # Arguments
    ///
    /// * `text` - what to show
    /// * `tone` - how loud it is
    #[must_use]
    pub fn new(text: impl Into<String>, tone: ReportTone) -> Self {
        Self {
            text: text.into(),
            tone,
        }
    }
}

/// The content of a tab that is built on demand: ready-to-draw lines.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Lines, top to bottom.
    pub lines: Vec<ReportLine>,
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
