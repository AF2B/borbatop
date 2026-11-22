//! What the person can press, and what the screen asks the program to do about it.

use borbatop_core::{Nice, Pid, ProcessId};

use crate::model::{ContainersView, PacmanView, Report};

/// A key press, independent of any terminal library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// A printable character.
    Char(char),
    /// Arrow up.
    Up,
    /// Arrow down.
    Down,
    /// Arrow left.
    Left,
    /// Arrow right.
    Right,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
    /// Home.
    Home,
    /// End.
    End,
    /// Enter.
    Enter,
    /// Escape.
    Escape,
    /// Backspace.
    Backspace,
    /// Tab.
    Tab,
    /// Shift+Tab.
    BackTab,
}

/// A screen of the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    /// Process table.
    Processes,
    /// systemd units.
    Services,
    /// Arch package manager report.
    Pacman,
    /// Network, disks and mounts.
    Io,
    /// Open sockets and the processes that own them.
    Connections,
    /// Startup time per unit.
    Boot,
    /// Warnings and errors of the system journal.
    Logs,
    /// Graphics cards.
    Gpu,
    /// Containers and the processes in them.
    Containers,
}

impl Tab {
    /// Every tab, in display order.
    pub const ALL: [Self; 9] = [
        Self::Processes,
        Self::Services,
        Self::Pacman,
        Self::Io,
        Self::Connections,
        Self::Boot,
        Self::Logs,
        Self::Gpu,
        Self::Containers,
    ];

    /// Name shown in the tab bar.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Processes => "Processes",
            Self::Services => "Services",
            Self::Pacman => "Pacman",
            Self::Io => "Network · Disk",
            Self::Connections => "Connections",
            Self::Boot => "Boot",
            Self::Logs => "Logs",
            Self::Gpu => "GPU",
            Self::Containers => "Containers",
        }
    }

    /// Whether the tab shows a [`crate::Report`] loaded on demand.
    #[must_use]
    pub const fn is_report(self) -> bool {
        match self {
            Self::Connections | Self::Boot | Self::Logs | Self::Gpu => true,
            Self::Processes | Self::Services | Self::Pacman | Self::Io | Self::Containers => false,
        }
    }

    /// Lower-case name for file names (`network-disk`).
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Processes => "processes",
            Self::Services => "services",
            Self::Pacman => "pacman",
            Self::Io => "network-disk",
            Self::Connections => "connections",
            Self::Boot => "boot",
            Self::Logs => "logs",
            Self::Gpu => "gpu",
            Self::Containers => "containers",
        }
    }

    /// Zero-based position in [`Tab::ALL`].
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Processes => 0,
            Self::Services => 1,
            Self::Pacman => 2,
            Self::Io => 3,
            Self::Connections => 4,
            Self::Boot => 5,
            Self::Logs => 6,
            Self::Gpu => 7,
            Self::Containers => 8,
        }
    }

    /// The tab at `index`, wrapping around in both directions.
    ///
    /// # Arguments
    ///
    /// * `index` - zero-based position, taken modulo the number of tabs
    #[must_use]
    pub fn wrapping(index: isize) -> Self {
        let count = isize::try_from(Self::ALL.len()).unwrap_or(1);
        let position = usize::try_from(index.rem_euclid(count)).unwrap_or(0);
        Self::ALL.get(position).copied().unwrap_or(Self::Processes)
    }
}

/// Column the process table is sorted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    /// CPU usage.
    Cpu,
    /// Resident memory.
    Memory,
    /// Process identifier.
    Pid,
    /// Command line.
    Command,
}

impl SortKey {
    /// Number of sortable columns.
    pub const COUNT: usize = 4;

    /// The next column in the cycle.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Cpu => Self::Memory,
            Self::Memory => Self::Pid,
            Self::Pid => Self::Command,
            Self::Command => Self::Cpu,
        }
    }

    /// Whether the largest value comes first by default.
    #[must_use]
    pub const fn descending_by_default(self) -> bool {
        match self {
            Self::Cpu | Self::Memory => true,
            Self::Pid | Self::Command => false,
        }
    }

    /// Column title.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU%",
            Self::Memory => "MEM",
            Self::Pid => "PID",
            Self::Command => "COMMAND",
        }
    }
}

/// A signal offered in the dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalChoice {
    /// Ask the process to finish cleanly.
    Terminate,
    /// Ask the process to reload its configuration.
    Hangup,
    /// End the process immediately.
    Kill,
}

impl SignalChoice {
    /// Choices in the order the dialog lists them.
    pub const ALL: [Self; 3] = [Self::Terminate, Self::Hangup, Self::Kill];

    /// Signal name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Terminate => "SIGTERM",
            Self::Hangup => "SIGHUP",
            Self::Kill => "SIGKILL",
        }
    }

    /// What the signal does, in one phrase.
    #[must_use]
    pub const fn effect(self) -> &'static str {
        match self {
            Self::Terminate => "finish cleanly",
            Self::Hangup => "reload configuration",
            Self::Kill => "end now, no cleanup",
        }
    }
}

/// An action offered for a systemd unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceAction {
    /// Stop and start the unit again.
    Restart,
    /// Stop the unit.
    Stop,
    /// Start the unit.
    Start,
}

impl ServiceAction {
    /// Actions in the order the dialog lists them.
    pub const ALL: [Self; 3] = [Self::Restart, Self::Stop, Self::Start];

    /// Action name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Restart => "restart",
            Self::Stop => "stop",
            Self::Start => "start",
        }
    }

    /// What the action does, in one phrase.
    #[must_use]
    pub const fn effect(self) -> &'static str {
        match self {
            Self::Restart => "stop, then start again",
            Self::Stop => "stop it now",
            Self::Start => "start it now",
        }
    }
}

/// Which way to move through a recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seek {
    /// One sample back.
    Backward,
    /// One sample forward.
    Forward,
}

/// An action offered for a container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerChoice {
    /// Stop and start again.
    Restart,
    /// Stop it.
    Stop,
    /// Start a stopped container.
    Start,
    /// Delete a stopped container.
    Remove,
}

impl ContainerChoice {
    /// Choices in the order the dialog lists them.
    pub const ALL: [Self; 4] = [Self::Restart, Self::Stop, Self::Start, Self::Remove];

    /// Action name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Restart => "restart",
            Self::Stop => "stop",
            Self::Start => "start",
            Self::Remove => "remove",
        }
    }

    /// What the action does, in one phrase.
    #[must_use]
    pub const fn effect(self) -> &'static str {
        match self {
            Self::Restart => "stop, then start again",
            Self::Stop => "stop it now",
            Self::Start => "start it now",
            Self::Remove => "delete it (must be stopped)",
        }
    }
}

/// What the person chose to update in the Pacman tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdatePlan {
    /// A full system upgrade of the repositories.
    Repositories,
    /// Every pending AUR upgrade of one helper.
    AurAll {
        /// Helper to run, for example `yay`.
        helper: String,
    },
    /// Chosen AUR packages of one helper.
    AurPackages {
        /// Helper to run, for example `yay`.
        helper: String,
        /// Packages to upgrade; never empty.
        packages: Vec<String>,
    },
    /// A snapshot of the system.
    Snapshot {
        /// Snapshot tool to run, `snapper` or `timeshift`.
        tool: String,
    },
    /// A snapshot, then the full repository upgrade.
    RepositoriesAfterSnapshot {
        /// Snapshot tool to run, `snapper` or `timeshift`.
        tool: String,
    },
    /// Merge the `.pacnew` files with `pacdiff`.
    MergeConfigs,
    /// Restart systemd units.
    RestartUnits {
        /// Units to restart; never empty.
        units: Vec<String>,
    },
}

/// What the Update Doctor looks at: the Pacman report as it is on screen, and the services that
/// still run with a replaced library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorRequest {
    /// The Pacman report.
    pub pacman: PacmanView,
    /// Systemd services that map a replaced library; restarting them picks up the new one.
    pub stale_units: Vec<String>,
}

/// How the process table was ordered when it was exported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportOrder {
    /// Column the table was sorted by.
    pub column: SortKey,
    /// Largest value first.
    pub descending: bool,
    /// Rows were arranged as a tree, so the column did not apply.
    pub tree: bool,
}

/// What one tab holds, as the person was seeing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportContent {
    /// Processes that passed the filter, in display order.
    Processes(Vec<Pid>),
    /// Units that passed the filter, in display order.
    Services(Vec<String>),
    /// The Pacman report.
    Pacman(Box<PacmanView>),
    /// Network, disks and mounts of the current sample.
    Io,
    /// The containers, with their state and usage.
    Containers(Box<ContainersView>),
    /// The report of an on-demand tab.
    Report(Report),
}

/// What to write when the person asks to export the tab being shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRequest {
    /// The tab being exported.
    pub tab: Tab,
    /// Filter text in effect, empty when none.
    pub filter: String,
    /// Ordering of the process table; `None` on other tabs.
    pub order: Option<ExportOrder>,
    /// The content itself.
    pub content: ExportContent,
}

/// What the screen asks the program to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Leave the program.
    Quit,
    /// Step through a recording; ignored while showing live data.
    Seek(Seek),
    /// Deliver a signal to a process.
    SendSignal {
        /// Process to signal: PID plus start time, so a reused PID is never hit.
        id: ProcessId,
        /// Signal to send.
        signal: SignalChoice,
    },
    /// Change the niceness of a process.
    SetNice {
        /// Process to change: PID plus start time.
        id: ProcessId,
        /// New niceness.
        nice: Nice,
    },
    /// Restrict a process to some CPUs.
    SetAffinity {
        /// Process to change: PID plus start time.
        id: ProcessId,
        /// Indices of the CPUs the process may use; never empty.
        cpus: Vec<usize>,
    },
    /// Start, stop or restart a unit.
    ControlUnit {
        /// Unit to act on.
        unit: String,
        /// What to do.
        action: ServiceAction,
    },
    /// Write the tab being shown to a JSON file.
    Export(Box<ExportRequest>),
    /// Run an update on the real terminal, where passwords and confirmations can be answered.
    RunUpdate(UpdatePlan),
    /// Check what to know before and after an update: news, disk, snapshot, restarts, configs.
    LoadDoctor(Box<DoctorRequest>),
    /// Show what changed between each `.pacnew` file and the configuration it belongs to.
    LoadPacnewDiffs(Vec<String>),
    /// Show everything known about one process.
    LoadDetail {
        /// Process to inspect: PID plus start time.
        id: ProcessId,
        /// Short name for the overlay title.
        label: String,
    },
    /// Build the report of a tab that loads on demand.
    LoadReport(Tab),
    /// List the containers of every installed runtime.
    LoadContainers,
    /// Start, stop, restart or remove a container.
    ControlContainer {
        /// Runtime that owns the container.
        runtime: String,
        /// Container identifier.
        id: String,
        /// What to do.
        action: ContainerChoice,
    },
    /// Show the latest log lines of a container.
    LoadContainerLogs {
        /// Runtime that owns the container.
        runtime: String,
        /// Container identifier.
        id: String,
        /// Container name, for the overlay title.
        name: String,
    },
    /// Rebuild the Pacman report.
    RefreshPacman,
    /// Fetch the journal of a unit.
    LoadLogs {
        /// Unit whose journal is wanted.
        unit: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_wrap_in_both_directions() {
        assert_eq!(Tab::wrapping(0), Tab::Processes);
        assert_eq!(Tab::wrapping(-1), Tab::Containers);
        assert_eq!(Tab::wrapping(9), Tab::Processes);
        assert_eq!(Tab::wrapping(11), Tab::Pacman);
        assert_eq!(Tab::ALL.iter().filter(|tab| tab.is_report()).count(), 4);
        for tab in Tab::ALL {
            assert_eq!(Tab::ALL.get(tab.index()), Some(&tab));
        }
    }

    #[test]
    fn sort_cycles_through_every_column() {
        let mut key = SortKey::Cpu;
        let mut seen = Vec::new();
        for _ in 0..4 {
            seen.push(key);
            key = key.next();
        }
        assert_eq!(
            seen,
            [
                SortKey::Cpu,
                SortKey::Memory,
                SortKey::Pid,
                SortKey::Command
            ]
        );
        assert_eq!(key, SortKey::Cpu);
    }
}
