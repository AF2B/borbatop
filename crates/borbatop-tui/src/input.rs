//! What the person can press, and what the screen asks the program to do about it.

use borbatop_core::{Nice, ProcessId};

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
            Self::Connections | Self::Boot | Self::Logs | Self::Gpu | Self::Containers => true,
            Self::Processes | Self::Services | Self::Pacman | Self::Io => false,
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
    /// Show everything known about one process.
    LoadDetail {
        /// Process to inspect: PID plus start time.
        id: ProcessId,
        /// Short name for the overlay title.
        label: String,
    },
    /// Build the report of a tab that loads on demand.
    LoadReport(Tab),
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
        assert_eq!(Tab::ALL.iter().filter(|tab| tab.is_report()).count(), 5);
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
