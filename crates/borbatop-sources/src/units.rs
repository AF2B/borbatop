//! systemd units and their logs, through `systemctl` and `journalctl`.

use borbatop_core::{UnitList, parse_systemctl_units};

use crate::command::run;
use crate::error::CommandError;

const MAX_UNIT_NAME_CHARS: usize = 256;
const UNIT_NAME_PUNCTUATION: [char; 5] = ['-', '_', '.', '@', ':'];

/// Lists services and timers, loaded or not.
///
/// # Errors
///
/// Returns [`CommandError`] when `systemctl` cannot be started or fails.
pub fn list_units() -> Result<UnitList, CommandError> {
    let output = run(
        "systemctl",
        &[
            "list-units",
            "--all",
            "--plain",
            "--no-legend",
            "--no-pager",
            "--type=service,timer",
        ],
        &[],
    )?;
    Ok(parse_systemctl_units(&output))
}

/// The most recent journal lines of one unit.
///
/// The journal can hold personal data: show it to the person at the keyboard, never log it.
///
/// # Arguments
///
/// * `unit` - unit name as `systemctl` printed it
/// * `lines` - how many of the latest lines to return
///
/// # Errors
///
/// * [`CommandError::InvalidUnitName`] when `unit` has characters no unit name can have.
/// * [`CommandError::Spawn`] / [`CommandError::Failed`] when `journalctl` cannot run.
pub fn unit_logs(unit: &str, lines: u16) -> Result<String, CommandError> {
    if !is_valid_unit_name(unit) {
        return Err(CommandError::InvalidUnitName(unit.to_owned()));
    }

    let count = lines.to_string();
    run(
        "journalctl",
        &[
            "--unit",
            unit,
            "--lines",
            &count,
            "--no-pager",
            "--output",
            "short",
        ],
        &[],
    )
}

fn is_valid_unit_name(unit: &str) -> bool {
    !unit.is_empty()
        && unit.chars().count() <= MAX_UNIT_NAME_CHARS
        && !unit.starts_with('-')
        && unit.chars().all(|character| {
            character.is_ascii_alphanumeric() || UNIT_NAME_PUNCTUATION.contains(&character)
        })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn accepts_real_unit_names() {
        assert!(is_valid_unit_name("sshd.service"));
        assert!(is_valid_unit_name("user@1000.service"));
        assert!(is_valid_unit_name(
            "systemd-fsck@dev-disk-by\\x2duuid.service"
                .replace('\\', "")
                .as_str()
        ));
    }

    #[test]
    fn rejects_option_injection_and_junk() {
        assert!(!is_valid_unit_name(""));
        assert!(!is_valid_unit_name("--output=json"));
        assert!(!is_valid_unit_name("a b"));
        assert!(!is_valid_unit_name("x;rm"));
        assert!(!is_valid_unit_name(&"a".repeat(300)));
    }

    #[test]
    fn invalid_name_never_reaches_the_shell() {
        let error = unit_logs("-n 1", 10).unwrap_err();
        assert!(matches!(error, CommandError::InvalidUnitName(_)));
    }

    #[test]
    fn lists_the_units_of_this_machine() {
        if !std::path::Path::new("/run/systemd/system").exists() {
            return; // not booted with systemd (container, chroot)
        }
        let list = list_units().unwrap();
        assert_ne!(list.units(), []);
    }
}
