//! Containers, as the installed runtimes list them.

use borbatop_core::{ContainerInfo, parse_container_list};

use crate::command::run;
use crate::error::{CommandError, Reading, SourceError};

const RUNTIMES: [&str; 2] = ["podman", "docker"];
const LIST_FORMAT: &str = "{{.ID}}\t{{.Names}}\t{{.Image}}\t{{.Status}}";

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

    /// Running containers, or why the runtime could not list them (daemon stopped, no permission).
    #[must_use]
    pub const fn containers(&self) -> &Reading<Vec<ContainerInfo>> {
        &self.containers
    }
}

/// Asks every installed container runtime for its running containers.
///
/// A runtime that is not installed is left out; one that is installed but fails is listed with
/// its error.
#[must_use]
pub fn list_containers() -> Vec<RuntimeListing> {
    RUNTIMES
        .iter()
        .filter_map(
            |&runtime| match run(runtime, &["ps", "--format", LIST_FORMAT], &[]) {
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
}
