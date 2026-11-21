//! Containers, as the installed runtimes list them.

use std::io::{self, Read};
use std::process::Command;

use borbatop_core::{ContainerInfo, parse_container_list};

use crate::command::run;
use crate::error::{CommandError, Reading, SourceError};

const RUNTIMES: [&str; 2] = ["podman", "docker"];
const LIST_FORMAT: &str = "{{.ID}}\t{{.Names}}\t{{.Image}}\t{{.Status}}\t{{.State}}";
const MAX_CONTAINER_CHARS: usize = 128;
const CONTAINER_PUNCTUATION: [char; 3] = ['_', '.', '-'];
const STDERR_CONTEXT_CHARS: usize = 200;

/// What one container runtime lists.
#[derive(Debug)]
pub struct RuntimeListing {
    runtime: &'static str,
    containers: Reading<Vec<ContainerInfo>>,
}

impl RuntimeListing {
    /// A listing from a runtime's answer.
    ///
    /// # Arguments
    ///
    /// * `runtime` - the program that answered
    /// * `containers` - what it listed, or why it could not
    #[must_use]
    pub const fn new(runtime: &'static str, containers: Reading<Vec<ContainerInfo>>) -> Self {
        Self {
            runtime,
            containers,
        }
    }

    /// The program that answered (`podman` or `docker`).
    #[must_use]
    pub const fn runtime(&self) -> &'static str {
        self.runtime
    }

    /// Containers, or why the runtime could not list them (daemon stopped, no permission).
    #[must_use]
    pub const fn containers(&self) -> &Reading<Vec<ContainerInfo>> {
        &self.containers
    }
}

/// Asks every installed container runtime for its containers, running or not.
///
/// A runtime that is not installed is left out; one that is installed but fails is listed with
/// its error.
#[must_use]
pub fn list_containers() -> Vec<RuntimeListing> {
    RUNTIMES
        .iter()
        .filter_map(
            |&runtime| match run(runtime, &["ps", "--all", "--format", LIST_FORMAT], &[]) {
                Ok(text) => Some(RuntimeListing {
                    runtime,
                    containers: Ok(parse_container_list(&text)),
                }),
                Err(CommandError::Spawn { source, .. })
                    if source.kind() == std::io::ErrorKind::NotFound =>
                {
                    None
                }
                Err(error) => Some(RuntimeListing {
                    runtime,
                    containers: Err(SourceError::Command(error)),
                }),
            },
        )
        .collect()
}

/// What can be done to a container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerAction {
    /// `start`: run a stopped container.
    Start,
    /// `stop`: ask the container to finish, then end it after the runtime's grace period.
    Stop,
    /// `restart`: stop, then start again.
    Restart,
    /// `rm`: delete a stopped container. A running one is refused by the runtime.
    Remove,
}

impl ContainerAction {
    /// The runtime sub-command.
    #[must_use]
    pub const fn verb(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::Remove => "rm",
        }
    }
}

/// The runtime this program knows by `name`, as a static string.
fn known_runtime(name: &str) -> Result<&'static str, CommandError> {
    RUNTIMES
        .iter()
        .copied()
        .find(|runtime| *runtime == name)
        .ok_or_else(|| CommandError::UnknownRuntime(name.to_owned()))
}

/// Container identifiers are hexadecimal; names may also have `_`, `.` and `-`, but none starts
/// with a dash, so an identifier can never be taken for an option.
fn is_valid_container(container: &str) -> bool {
    !container.is_empty()
        && container.chars().count() <= MAX_CONTAINER_CHARS
        && container.starts_with(|first: char| first.is_ascii_alphanumeric())
        && container.chars().all(|character| {
            character.is_ascii_alphanumeric() || CONTAINER_PUNCTUATION.contains(&character)
        })
}

fn checked(runtime: &str, container: &str) -> Result<&'static str, CommandError> {
    let runtime = known_runtime(runtime)?;
    if !is_valid_container(container) {
        return Err(CommandError::InvalidContainer(container.to_owned()));
    }
    Ok(runtime)
}

/// Starts, stops, restarts or removes one container.
///
/// # Arguments
///
/// * `runtime` - `podman` or `docker`
/// * `container` - identifier or name, as the runtime listed it
/// * `action` - what to do with it
///
/// # Errors
///
/// * [`CommandError::UnknownRuntime`] / [`CommandError::InvalidContainer`] for arguments no
///   runtime would accept; nothing is run.
/// * [`CommandError::Spawn`] / [`CommandError::Failed`] when the runtime cannot run or refuses
///   (for example: no permission on the Docker socket, removing a running container).
pub fn control_container(
    runtime: &str,
    container: &str,
    action: ContainerAction,
) -> Result<(), CommandError> {
    let runtime = checked(runtime, container)?;
    run(runtime, &[action.verb(), container], &[]).map(|_output| ())
}

/// The newest log lines of one container, standard output and standard error together.
///
/// Logs can hold personal data: show them to the person at the keyboard, never log them.
///
/// # Arguments
///
/// * `runtime` - `podman` or `docker`
/// * `container` - identifier or name, as the runtime listed it
/// * `lines` - how many of the latest lines to return
///
/// # Errors
///
/// Same as [`control_container`].
pub fn container_logs(runtime: &str, container: &str, lines: u16) -> Result<String, CommandError> {
    let runtime = checked(runtime, container)?;
    let spawn_error = |source| CommandError::Spawn {
        program: runtime,
        source,
    };

    // Containers write errors to standard error: one pipe for both keeps their order.
    let (mut reader, writer) = io::pipe().map_err(spawn_error)?;
    let mut command = Command::new(runtime);
    command
        .args(["logs", "--tail", &lines.to_string(), container])
        .stdout(writer.try_clone().map_err(spawn_error)?)
        .stderr(writer);
    let mut child = command.spawn().map_err(spawn_error)?;
    // The command holds the write ends; dropping it lets the read below see the end of the output.
    drop(command);

    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).map_err(spawn_error)?;
    let status = child.wait().map_err(spawn_error)?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    if status.success() {
        return Ok(text);
    }
    Err(CommandError::Failed {
        program: runtime,
        code: status.code(),
        stderr: text
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(STDERR_CONTEXT_CHARS)
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_installed_runtime_answers_with_a_list_or_an_error() {
        for listing in list_containers() {
            assert!(RUNTIMES.contains(&listing.runtime()));
            if let Ok(containers) = listing.containers() {
                assert!(containers.iter().all(|container| !container.id.is_empty()));
            }
        }
    }
    #[test]
    fn verbs_match_the_runtime_sub_commands() {
        assert_eq!(ContainerAction::Start.verb(), "start");
        assert_eq!(ContainerAction::Stop.verb(), "stop");
        assert_eq!(ContainerAction::Restart.verb(), "restart");
        assert_eq!(ContainerAction::Remove.verb(), "rm");
    }

    #[test]
    fn identifiers_and_names_are_validated_before_anything_runs() {
        for good in ["0a1b2c3d4e5f", "web", "my_app-1.2", "A9"] {
            assert!(is_valid_container(good), "{good}");
        }
        for bad in [
            "",
            "-f",
            "--all",
            "a b",
            "x;rm",
            "a/b",
            "$(id)",
            &"a".repeat(200),
        ] {
            assert!(!is_valid_container(bad), "{bad}");
        }
    }

    #[test]
    fn bad_input_never_reaches_a_runtime() {
        for action in [
            ContainerAction::Start,
            ContainerAction::Stop,
            ContainerAction::Remove,
        ] {
            assert!(matches!(
                control_container("docker", "--all", action),
                Err(CommandError::InvalidContainer(_))
            ));
        }
        assert!(matches!(
            control_container("rm", "web", ContainerAction::Stop),
            Err(CommandError::UnknownRuntime(_))
        ));
        assert!(matches!(
            container_logs("podman", "-x", 10),
            Err(CommandError::InvalidContainer(_))
        ));
    }

    #[test]
    fn logs_of_a_container_that_does_not_exist_fail_with_the_runtime_message() {
        for listing in list_containers() {
            let runtime = listing.runtime();
            let result = container_logs(runtime, "borbatop-no-such-container", 5);
            assert!(
                matches!(result, Err(CommandError::Failed { .. })),
                "{runtime}: {result:?}"
            );
        }
    }
}
