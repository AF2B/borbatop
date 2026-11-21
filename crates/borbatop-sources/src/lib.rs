//! Linux data sources: `/proc`, `/sys`, cgroup v2, `systemctl` and pacman.
//!
//! All I/O of borbatop lives here and is turned into the pure types of `borbatop-core`.

mod command;
mod error;
mod owners;
mod pacman;
mod proc;
mod signal;
mod statics;
mod system;
mod units;

pub use error::{CommandError, ParseCause, Reading, SourceError};
pub use owners::{FileOwners, PACMAN_LOCAL_DB};
pub use pacman::{
    KernelStatus, PacmanReport, PendingConfig, StaleProcess, StaleScan, collect_pacman_report,
};
pub use proc::{ProcCollector, ProcessFailure, ProcessFailureCause, ProcessRecord, Snapshot};
pub use signal::{Signal, SignalError, send_signal};
pub use statics::{NOT_AVAILABLE, ProcessStatics};
pub use system::{MountReport, MountUsage, SystemCollector, SystemSnapshot};
pub use units::{list_units, unit_logs};
