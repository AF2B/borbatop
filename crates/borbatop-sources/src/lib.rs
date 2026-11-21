//! Linux data sources: `/proc`, `/sys`, cgroup v2, `systemctl` and pacman.
//!
//! All I/O of borbatop lives here and is turned into the pure types of `borbatop-core`.

mod command;
mod connections;
mod containers;
mod detail;
mod diagnostics;
mod error;
mod gpu;
mod identity;
mod owners;
mod pacman;
mod proc;
mod schedule;
mod signal;
mod statics;
mod system;
mod units;
mod updater;

pub use connections::{Connection, ConnectionReport, SocketOwner, collect_connections};
pub use containers::{
    ContainerAction, RuntimeListing, container_logs, control_container, list_containers,
};
pub use detail::{OpenDescriptor, ProcessDetail, process_detail};
pub use diagnostics::{JournalLevel, startup_blame, startup_summary, system_journal};
pub use error::{CommandError, ParseCause, Reading, SourceError};
pub use gpu::{GpuReport, collect_gpus};
pub use owners::{FileOwners, PACMAN_LOCAL_DB};
pub use pacman::{
    AurUpgrades, KernelStatus, PacmanReport, PendingConfig, StaleProcess, StaleScan, UpgradeSource,
    collect_pacman_report,
};
pub use proc::{ProcCollector, ProcessFailure, ProcessFailureCause, ProcessRecord, Snapshot};
pub use schedule::{ScheduleError, set_affinity, set_nice};
pub use signal::{Signal, SignalError, send_signal};
pub use statics::{NOT_AVAILABLE, ProcessStatics};
pub use system::{MountReport, MountUsage, SystemCollector, SystemSnapshot};
pub use units::{UnitAction, control_unit, list_units, unit_logs};
pub use updater::{UpdateCommand, UpdateError, UpdateRequest, update_command};
