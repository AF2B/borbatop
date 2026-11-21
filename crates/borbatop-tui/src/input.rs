//! What the person can press, and what the screen asks the program to do about it.

use borbatop_core::ProcessId;

/// A key press, independent of any terminal library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// A printable character.
    Char(char),
    /// Arrow up.
    Up,
    /// Arrow down.
    Down,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    /// Process table.
    Processes,
    /// systemd units.
    Services,
    /// Arch package manager report.
    Pacman,
    /// Network, disks and mounts.
    Io,
}

impl Tab {
    /// Every tab, in display order.
    pub const ALL: [Self; 4] = [Self::Processes, Self::Services, Self::Pacman, Self::Io];

    /// Name shown in the tab bar.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Processes => "Processes",
            Self::Services => "Services",
            Self::Pacman => "Pacman",
            Self::Io => "Network · Disk",
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

/// What the screen asks the program to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Leave the program.
    Quit,
    /// Deliver a signal to a process.
    SendSignal {
        /// Process to signal: PID plus start time, so a reused PID is never hit.
        id: ProcessId,
        /// Signal to send.
        signal: SignalChoice,
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
        assert_eq!(Tab::wrapping(-1), Tab::Io);
        assert_eq!(Tab::wrapping(4), Tab::Processes);
        assert_eq!(Tab::wrapping(6), Tab::Pacman);
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
