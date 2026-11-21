//! Containers: recognizing a container's systemd scope and parsing `podman ps` / `docker ps`.

const SCOPE_SUFFIX: &str = ".scope";
const SCOPE_PREFIXES: [&str; 4] = ["libpod-", "docker-", "cri-containerd-", "crio-"];
const FIELD_SEPARATOR: char = '\t';
const RUNNING_STATE: &str = "running";

/// The container identifier in a scope unit name such as `libpod-<id>.scope`.
///
/// # Arguments
///
/// * `unit` - systemd unit that runs a process
///
/// # Returns
///
/// `None` when the unit is not the scope of a container started by Podman, Docker, containerd
/// or CRI-O under the systemd cgroup driver.
///
/// # Examples
///
/// ```
/// use borbatop_core::container_id_of_unit;
///
/// assert_eq!(container_id_of_unit("libpod-0a1b2c.scope"), Some("0a1b2c"));
/// assert_eq!(container_id_of_unit("sshd.service"), None);
/// ```
#[must_use]
pub fn container_id_of_unit(unit: &str) -> Option<&str> {
    let body = unit.strip_suffix(SCOPE_SUFFIX)?;
    SCOPE_PREFIXES
        .iter()
        .find_map(|prefix| body.strip_prefix(prefix))
        .filter(|id| !id.is_empty())
}

/// Whether a short identifier (as `ps` prints it) names the container with the full one.
///
/// # Arguments
///
/// * `full` - identifier from the scope unit
/// * `short` - identifier from the runtime's listing
#[must_use]
pub fn same_container(full: &str, short: &str) -> bool {
    !short.is_empty() && full.starts_with(short)
}

/// One row of `ps -a --format '{{.ID}}\t{{.Names}}\t{{.Image}}\t{{.Status}}\t{{.State}}'`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerInfo {
    /// Identifier, as the runtime prints it (usually the first 12 hexadecimal digits).
    pub id: String,
    /// Name given to the container.
    pub name: String,
    /// Image it was created from.
    pub image: String,
    /// Status text, for example `Up 3 hours`.
    pub status: String,
    /// Lifecycle state as the runtime names it (`running`, `exited`, `created`, ...).
    pub state: String,
}

impl ContainerInfo {
    /// Whether the container is running now.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.state == RUNNING_STATE
    }
}

/// Parses the tab-separated listing of a container runtime; lines with the wrong number of
/// fields are left out.
///
/// # Arguments
///
/// * `text` - standard output of `podman ps` or `docker ps` with the format above
#[must_use]
pub fn parse_container_list(text: &str) -> Vec<ContainerInfo> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split(FIELD_SEPARATOR);
            let info = ContainerInfo {
                id: fields.next()?.to_owned(),
                name: fields.next()?.to_owned(),
                image: fields.next()?.to_owned(),
                status: fields.next()?.to_owned(),
                state: fields.next()?.to_owned(),
            };
            (fields.next().is_none() && !info.id.is_empty()).then_some(info)
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn scopes_of_the_known_runtimes_yield_their_ids() {
        assert_eq!(container_id_of_unit("libpod-abc123.scope"), Some("abc123"));
        assert_eq!(container_id_of_unit("docker-abc123.scope"), Some("abc123"));
        assert_eq!(
            container_id_of_unit("cri-containerd-abc123.scope"),
            Some("abc123")
        );
        assert_eq!(container_id_of_unit("crio-abc123.scope"), Some("abc123"));
    }

    #[test]
    fn other_units_are_not_containers() {
        for unit in [
            "sshd.service",
            "libpod-abc123.service",
            "libpod-.scope",
            "app-x.scope",
            "",
        ] {
            assert_eq!(container_id_of_unit(unit), None, "{unit:?}");
        }
    }

    #[test]
    fn short_ids_match_by_prefix_and_empty_ones_match_nothing() {
        assert!(same_container("0a1b2c3d4e5f6789", "0a1b2c3d4e5f"));
        assert!(!same_container("0a1b2c3d4e5f6789", "ffffffffffff"));
        assert!(!same_container("0a1b2c3d4e5f6789", ""));
    }

    #[test]
    fn listings_parse_and_malformed_rows_are_left_out() {
        let rows = parse_container_list(
            "0a1b2c3d4e5f\tweb\tdocker.io/library/nginx:latest\tUp 3 hours\trunning\nbroken line\n\tno-id\timg\tUp\trunning\nabc\tdb\tpostgres\tUp 1 hour\trunning\textra\n9e8d7c6b5a49\told\talpine\tExited (0) 2 days ago\texited\n",
        );
        assert_eq!(rows.len(), 2);
        let web = rows.first().unwrap();
        assert_eq!(web.name, "web");
        assert_eq!(web.status, "Up 3 hours");
        assert!(web.is_running());
        let old = rows.get(1).unwrap();
        assert_eq!(old.state, "exited");
        assert!(
            !old.is_running(),
            "a stopped container is listed but not running"
        );
    }
}
