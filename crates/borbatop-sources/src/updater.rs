//! Turning "update this" into the exact command to run, after validating every argument.
//!
//! Nothing is executed here: the caller runs the command on the real terminal, where `sudo` and
//! the AUR helpers can ask for a password and for confirmation.

use std::error::Error;
use std::fmt;

const SUDO: &str = "sudo";
const PACMAN: &str = "pacman";
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
}

impl fmt::Display for UpdateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownHelper(name) => write!(formatter, "unknown AUR helper: {name:?}"),
            Self::NoPackages => write!(formatter, "choose at least one package"),
            Self::InvalidPackage(name) => write!(formatter, "invalid package name: {name:?}"),
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

/// The command that carries out `request`.
///
/// # Arguments
///
/// * `request` - what the person chose to update
///
/// # Errors
///
/// * [`UpdateError::UnknownHelper`] when the helper is neither `yay` nor `paru`.
/// * [`UpdateError::NoPackages`] when a package choice is empty.
/// * [`UpdateError::InvalidPackage`] for a package name that is not a valid Arch package name.
///
/// # Examples
///
/// ```
/// use borbatop_sources::{UpdateRequest, update_command};
///
/// let command = update_command(&UpdateRequest::Repositories).map_err(|error| error.to_string())?;
/// assert_eq!(command.to_string(), "sudo pacman -Syu");
/// # Ok::<(), String>(())
/// ```
pub fn update_command(request: &UpdateRequest) -> Result<UpdateCommand, UpdateError> {
    match request {
        UpdateRequest::Repositories => Ok(UpdateCommand {
            program: SUDO,
            arguments: vec![PACMAN.to_owned(), "-Syu".to_owned()],
        }),
        UpdateRequest::AurAll { helper } => Ok(UpdateCommand {
            program: known_helper(helper)?,
            arguments: vec!["-Sua".to_owned()],
        }),
        UpdateRequest::AurPackages { helper, packages } => {
            let program = known_helper(helper)?;
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
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn packages(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn repositories_are_always_a_full_system_upgrade() {
        let command = update_command(&UpdateRequest::Repositories).unwrap();
        assert_eq!(command.program(), "sudo");
        assert_eq!(command.arguments(), ["pacman", "-Syu"]);
        assert_eq!(command.to_string(), "sudo pacman -Syu");
    }

    #[test]
    fn aur_helpers_upgrade_everything_or_the_chosen_packages() {
        let all = update_command(&UpdateRequest::AurAll {
            helper: "paru".to_owned(),
        })
        .unwrap();
        assert_eq!(all.to_string(), "paru -Sua");

        let some = update_command(&UpdateRequest::AurPackages {
            helper: "yay".to_owned(),
            packages: packages(&["clojure-lsp-bin", "yay", "gcc13-libs"]),
        })
        .unwrap();
        assert_eq!(some.program(), "yay");
        assert_eq!(some.to_string(), "yay -S clojure-lsp-bin yay gcc13-libs");
    }

    #[test]
    fn only_known_helpers_are_run() {
        for helper in ["pikaur", "sh", "rm", "", "yay ", "/usr/bin/yay"] {
            assert_eq!(
                update_command(&UpdateRequest::AurAll {
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
                update_command(&UpdateRequest::AurPackages {
                    helper: "yay".to_owned(),
                    packages: packages(&["fine", bad]),
                }),
                Err(UpdateError::InvalidPackage(bad.to_owned()))
            );
        }
    }

    #[test]
    fn an_empty_choice_is_refused() {
        assert_eq!(
            update_command(&UpdateRequest::AurPackages {
                helper: "yay".to_owned(),
                packages: Vec::new(),
            }),
            Err(UpdateError::NoPackages)
        );
    }
}
