//! Turning "update this" into the exact command to run, after validating every argument.
//!
//! Nothing is executed here: the caller runs the command on the real terminal, where `sudo` and
//! the AUR helpers can ask for a password and for confirmation.

use std::error::Error;
use std::fmt;

use crate::doctor::SnapshotTool;
use crate::units::is_valid_unit_name;

const SUDO: &str = "sudo";
const PACMAN: &str = "pacman";
const PACDIFF: &str = "pacdiff";
const SNAPSHOT_NOTE: &str = "borbatop: before update";
const AUR_HELPERS: [&str; 2] = ["yay", "paru"];
const MAX_PACKAGE_CHARS: usize = 256;
const PACKAGE_PUNCTUATION: [char; 5] = ['@', '.', '_', '+', '-'];

/// What the person chose to update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateRequest {
    /// Every pending repository upgrade, as one full system upgrade (`pacman -Syu`).
    ///
    /// Never one package at a time: upgrading a single package leaves the system partially
    /// upgraded, which Arch does not support.
    Repositories,
    /// Every pending AUR upgrade of one helper.
    AurAll {
        /// `yay` or `paru`.
        helper: String,
    },
    /// Chosen AUR packages of one helper.
    AurPackages {
        /// `yay` or `paru`.
        helper: String,
        /// Package names to upgrade; must not be empty.
        packages: Vec<String>,
    },
    /// A snapshot of the system, taken by `snapper` or `timeshift`.
    Snapshot {
        /// Program name of the tool.
        tool: String,
    },
    /// A snapshot, then the full repository upgrade; the upgrade runs only if the snapshot worked.
    RepositoriesAfterSnapshot {
        /// Program name of the tool.
        tool: String,
    },
    /// Merge the `.pacnew` files with `pacdiff`.
    MergeConfigs,
    /// Restart systemd units, for example the services that still use a replaced library.
    RestartUnits {
        /// Unit names; must not be empty.
        units: Vec<String>,
    },
}

/// Why an update request cannot become a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
    /// The helper is not one this program knows how to drive.
    UnknownHelper(String),
    /// No package was chosen.
    NoPackages,
    /// A package name that no package can have (or that could be taken for an option).
    InvalidPackage(String),
    /// The snapshot tool is neither `snapper` nor `timeshift`.
    UnknownSnapshotTool(String),
    /// A unit name that no systemd unit can have.
    InvalidUnit(String),
    /// No unit was chosen.
    NoUnits,
}

impl fmt::Display for UpdateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownHelper(name) => write!(formatter, "unknown AUR helper: {name:?}"),
            Self::NoPackages => write!(formatter, "choose at least one package"),
            Self::InvalidPackage(name) => write!(formatter, "invalid package name: {name:?}"),
            Self::UnknownSnapshotTool(name) => write!(formatter, "unknown snapshot tool: {name:?}"),
            Self::InvalidUnit(name) => write!(formatter, "invalid unit name: {name:?}"),
            Self::NoUnits => write!(formatter, "choose at least one unit"),
        }
    }
}

impl Error for UpdateError {}

/// A program and its arguments, ready to run without a shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCommand {
    program: &'static str,
    arguments: Vec<String>,
}

impl UpdateCommand {
    /// The program to run (`sudo`, `yay` or `paru`).
    #[must_use]
    pub const fn program(&self) -> &'static str {
        self.program
    }

    /// Its arguments, in order.
    #[must_use]
    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }
}

impl fmt::Display for UpdateCommand {
    /// The command as the person would type it, for the screen.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.program)?;
        for argument in &self.arguments {
            write!(formatter, " {argument}")?;
        }
        Ok(())
    }
}

fn known_helper(name: &str) -> Result<&'static str, UpdateError> {
    AUR_HELPERS
        .iter()
        .copied()
        .find(|helper| *helper == name)
        .ok_or_else(|| UpdateError::UnknownHelper(name.to_owned()))
}

/// Arch package names are lower-case letters, digits and `@ . _ + -`, and never start with a dash
/// or a dot.
fn is_valid_package(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_PACKAGE_CHARS
        && name.starts_with(|first: char| first.is_ascii_alphanumeric())
        && name.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || PACKAGE_PUNCTUATION.contains(&character)
        })
}

fn snapshot_command(tool: &str) -> Result<UpdateCommand, UpdateError> {
    let tool = SnapshotTool::from_program(tool)
        .ok_or_else(|| UpdateError::UnknownSnapshotTool(tool.to_owned()))?;
    let arguments = match tool {
        SnapshotTool::Snapper => vec![
            "snapper".to_owned(),
            "create".to_owned(),
            "--description".to_owned(),
            SNAPSHOT_NOTE.to_owned(),
        ],
        SnapshotTool::Timeshift => vec![
            "timeshift".to_owned(),
            "--create".to_owned(),
            "--comments".to_owned(),
            SNAPSHOT_NOTE.to_owned(),
            "--tags".to_owned(),
            "D".to_owned(),
        ],
    };
    Ok(UpdateCommand {
        program: SUDO,
        arguments,
    })
}

fn repositories_command() -> UpdateCommand {
    UpdateCommand {
        program: SUDO,
        arguments: vec![PACMAN.to_owned(), "-Syu".to_owned()],
    }
}

fn aur_command(helper: &str, packages: Option<&[String]>) -> Result<UpdateCommand, UpdateError> {
    let program = known_helper(helper)?;
    let Some(packages) = packages else {
        return Ok(UpdateCommand {
            program,
            arguments: vec!["-Sua".to_owned()],
        });
    };
    if packages.is_empty() {
        return Err(UpdateError::NoPackages);
    }
    if let Some(bad) = packages.iter().find(|name| !is_valid_package(name)) {
        return Err(UpdateError::InvalidPackage(bad.clone()));
    }
    let mut arguments = Vec::with_capacity(packages.len().saturating_add(1));
    arguments.push("-S".to_owned());
    arguments.extend(packages.iter().cloned());
    Ok(UpdateCommand { program, arguments })
}

fn restart_command(units: &[String]) -> Result<UpdateCommand, UpdateError> {
    if units.is_empty() {
        return Err(UpdateError::NoUnits);
    }
    if let Some(bad) = units.iter().find(|unit| !is_valid_unit_name(unit)) {
        return Err(UpdateError::InvalidUnit(bad.clone()));
    }
    let mut arguments = vec!["systemctl".to_owned(), "restart".to_owned()];
    arguments.extend(units.iter().cloned());
    Ok(UpdateCommand {
        program: SUDO,
        arguments,
    })
}

/// The commands that carry out `request`, in the order they must run. The caller stops at the
/// first one that fails.
///
/// # Arguments
///
/// * `request` - what the person chose to do
///
/// # Errors
///
/// * [`UpdateError::UnknownHelper`] when the helper is neither `yay` nor `paru`.
/// * [`UpdateError::UnknownSnapshotTool`] when the tool is neither `snapper` nor `timeshift`.
/// * [`UpdateError::NoPackages`] / [`UpdateError::NoUnits`] when a choice is empty.
/// * [`UpdateError::InvalidPackage`] / [`UpdateError::InvalidUnit`] for a name that is not valid
///   (or that could be taken for an option).
///
/// # Examples
///
/// ```
/// use borbatop_sources::{UpdateRequest, update_commands};
///
/// let commands = update_commands(&UpdateRequest::Repositories).map_err(|error| error.to_string())?;
/// assert_eq!(commands.first().map(ToString::to_string), Some("sudo pacman -Syu".to_owned()));
/// # Ok::<(), String>(())
/// ```
pub fn update_commands(request: &UpdateRequest) -> Result<Vec<UpdateCommand>, UpdateError> {
    match request {
        UpdateRequest::Repositories => Ok(vec![repositories_command()]),
        UpdateRequest::AurAll { helper } => Ok(vec![aur_command(helper, None)?]),
        UpdateRequest::AurPackages { helper, packages } => {
            Ok(vec![aur_command(helper, Some(packages))?])
        }
        UpdateRequest::Snapshot { tool } => Ok(vec![snapshot_command(tool)?]),
        UpdateRequest::RepositoriesAfterSnapshot { tool } => {
            Ok(vec![snapshot_command(tool)?, repositories_command()])
        }
        UpdateRequest::MergeConfigs => Ok(vec![UpdateCommand {
            program: SUDO,
            arguments: vec![PACDIFF.to_owned()],
        }]),
        UpdateRequest::RestartUnits { units } => Ok(vec![restart_command(units)?]),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn packages(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn texts(request: &UpdateRequest) -> Vec<String> {
        update_commands(request)
            .unwrap()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    #[test]
    fn repositories_are_always_a_full_system_upgrade() {
        assert_eq!(texts(&UpdateRequest::Repositories), ["sudo pacman -Syu"]);
        let command = update_commands(&UpdateRequest::Repositories)
            .unwrap()
            .remove(0);
        assert_eq!(command.program(), "sudo");
        assert_eq!(command.arguments(), ["pacman", "-Syu"]);
    }

    #[test]
    fn aur_helpers_upgrade_everything_or_the_chosen_packages() {
        assert_eq!(
            texts(&UpdateRequest::AurAll {
                helper: "paru".to_owned()
            }),
            ["paru -Sua"]
        );
        assert_eq!(
            texts(&UpdateRequest::AurPackages {
                helper: "yay".to_owned(),
                packages: packages(&["clojure-lsp-bin", "yay", "gcc13-libs"]),
            }),
            ["yay -S clojure-lsp-bin yay gcc13-libs"]
        );
    }

    #[test]
    fn a_snapshot_comes_before_the_upgrade_and_each_tool_has_its_own_command() {
        assert_eq!(
            texts(&UpdateRequest::RepositoriesAfterSnapshot {
                tool: "snapper".to_owned()
            }),
            [
                "sudo snapper create --description borbatop: before update",
                "sudo pacman -Syu"
            ]
        );
        assert_eq!(
            texts(&UpdateRequest::Snapshot {
                tool: "timeshift".to_owned()
            }),
            ["sudo timeshift --create --comments borbatop: before update --tags D"]
        );
    }

    #[test]
    fn only_known_snapshot_tools_are_run() {
        for tool in ["btrfs", "sh", "", "snapper ", "/usr/bin/snapper"] {
            assert_eq!(
                update_commands(&UpdateRequest::Snapshot {
                    tool: tool.to_owned()
                }),
                Err(UpdateError::UnknownSnapshotTool(tool.to_owned())),
                "{tool:?}"
            );
        }
    }

    #[test]
    fn config_merging_and_service_restarts_are_plain_commands() {
        assert_eq!(texts(&UpdateRequest::MergeConfigs), ["sudo pacdiff"]);
        assert_eq!(
            texts(&UpdateRequest::RestartUnits {
                units: packages(&["sshd.service", "user@1000.service"])
            }),
            ["sudo systemctl restart sshd.service user@1000.service"]
        );
    }

    #[test]
    fn unit_names_are_validated_and_an_empty_choice_is_refused() {
        for bad in ["--now", "a b", "x;rm", "", "-f"] {
            assert_eq!(
                update_commands(&UpdateRequest::RestartUnits {
                    units: packages(&["ok.service", bad])
                }),
                Err(UpdateError::InvalidUnit(bad.to_owned())),
                "{bad:?}"
            );
        }
        assert_eq!(
            update_commands(&UpdateRequest::RestartUnits { units: Vec::new() }),
            Err(UpdateError::NoUnits)
        );
    }

    #[test]
    fn only_known_helpers_are_run() {
        for helper in ["pikaur", "sh", "rm", "", "yay ", "/usr/bin/yay"] {
            assert_eq!(
                update_commands(&UpdateRequest::AurAll {
                    helper: helper.to_owned()
                }),
                Err(UpdateError::UnknownHelper(helper.to_owned())),
                "{helper:?}"
            );
        }
    }

    #[test]
    fn package_names_are_validated_so_none_can_become_an_option_or_a_shell_word() {
        for good in [
            "firefox",
            "lib32-glibc",
            "python-pip3",
            "gtk+",
            "a.b_c@d",
            "x11",
        ] {
            assert!(is_valid_package(good), "{good}");
        }
        for bad in [
            "",
            "-S",
            "--noconfirm",
            ".hidden",
            "Upper",
            "a b",
            "x;rm",
            "$(id)",
            "a/b",
            &"a".repeat(300),
        ] {
            assert!(!is_valid_package(bad), "{bad}");
            assert_eq!(
                update_commands(&UpdateRequest::AurPackages {
                    helper: "yay".to_owned(),
                    packages: packages(&["fine", bad]),
                }),
                Err(UpdateError::InvalidPackage(bad.to_owned()))
            );
        }
    }

    #[test]
    fn an_empty_package_choice_is_refused() {
        assert_eq!(
            update_commands(&UpdateRequest::AurPackages {
                helper: "yay".to_owned(),
                packages: Vec::new(),
            }),
            Err(UpdateError::NoPackages)
        );
    }
}
