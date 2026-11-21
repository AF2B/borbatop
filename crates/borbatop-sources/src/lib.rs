//! Linux data sources: `/proc`, `/sys`, cgroup v2 and `systemctl`.
//!
//! All I/O of borbatop lives here and is turned into the pure types of `borbatop-core`.

mod proc;

pub use proc::{ProcCollector, ProcessFailure, ProcessFailureCause, Snapshot, SourceError};
