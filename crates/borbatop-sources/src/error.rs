//! Errors of every data source, and the result type of optional readings.

use std::error::Error;
use std::fmt;
use std::io;
use std::path::PathBuf;

use borbatop_core::{CpuStatError, Pid, ZeroPageSize};

/// Boxed cause of a parse failure, kept so the original error is never flattened to text.
pub type ParseCause = Box<dyn Error + Send + Sync + 'static>;

/// Failure of one data source.
#[derive(Debug)]
pub enum SourceError {
    /// A file or directory could not be read.
    Io {
        /// Path that failed.
        path: PathBuf,
        /// Underlying OS error.
        source: io::Error,
    },
    /// `/proc/stat` could not be parsed.
    CpuStat(CpuStatError),
    /// A file was read but its contents are malformed.
    Parse {
        /// Path whose contents were malformed.
        path: PathBuf,
        /// Why the parser rejected it.
        source: ParseCause,
    },
    /// The kernel reported a page size of zero.
    PageSize(ZeroPageSize),
    /// An external command could not be run or failed.
    Command(CommandError),
    /// The process exited, or its PID now belongs to a different process.
    ProcessGone(Pid),
}

impl SourceError {
    /// Builds an [`SourceError::Io`] for `path`.
    pub(crate) fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    /// Builds a [`SourceError::Parse`] for `path` from any parser error.
    pub(crate) fn parse(
        path: impl Into<PathBuf>,
        source: impl Error + Send + Sync + 'static,
    ) -> Self {
        Self::Parse {
            path: path.into(),
            source: Box::new(source),
        }
    }
}

impl fmt::Display for SourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(formatter, "cannot read {}: {source}", path.display())
            }
            Self::CpuStat(error) => write!(formatter, "invalid /proc/stat: {error}"),
            Self::Parse { path, source } => {
                write!(formatter, "invalid {}: {source}", path.display())
            }
            Self::PageSize(error) => error.fmt(formatter),
            Self::Command(error) => error.fmt(formatter),
            Self::ProcessGone(pid) => write!(formatter, "process {pid} is gone"),
        }
    }
}

impl Error for SourceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::CpuStat(error) => Some(error),
            Self::Parse { source, .. } => Some(source.as_ref()),
            Self::PageSize(error) => Some(error),
            Self::Command(error) => Some(error),
            Self::ProcessGone(_) => None,
        }
    }
}

/// An external command (`systemctl`, `journalctl`, `pacman`) that did not complete.
#[derive(Debug)]
pub enum CommandError {
    /// The program could not be started (not installed, not executable).
    Spawn {
        /// Program that failed to start.
        program: &'static str,
        /// Underlying OS error.
        source: io::Error,
    },
    /// The program ran and exited with an unexpected status.
    Failed {
        /// Program that failed.
        program: &'static str,
        /// Exit code, or `None` when a signal ended it.
        code: Option<i32>,
        /// First line of its standard error, for context.
        stderr: String,
    },
    /// A unit name contains characters that no systemd unit can have.
    InvalidUnitName(String),
    /// A container identifier or name contains characters no container can have.
    InvalidContainer(String),
    /// The container runtime is not one this program knows how to drive.
    UnknownRuntime(String),
}

impl fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn { program, source } => write!(formatter, "cannot run {program}: {source}"),
            Self::Failed {
                program,
                code: Some(code),
                stderr,
            } => write!(formatter, "{program} exited with {code}: {stderr}"),
            Self::Failed {
                program,
                code: None,
                stderr,
            } => write!(formatter, "{program} was killed: {stderr}"),
            Self::InvalidUnitName(name) => write!(formatter, "invalid unit name: {name:?}"),
            Self::InvalidContainer(name) => write!(formatter, "invalid container: {name:?}"),
            Self::UnknownRuntime(name) => write!(formatter, "unknown container runtime: {name:?}"),
        }
    }
}

impl Error for CommandError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Spawn { source, .. } => Some(source),
            Self::Failed { .. }
            | Self::InvalidUnitName(_)
            | Self::InvalidContainer(_)
            | Self::UnknownRuntime(_) => None,
        }
    }
}

/// A reading that may be unavailable (no PSI in the kernel, no hwmon sensor, not Arch).
pub type Reading<T> = Result<T, SourceError>;
