//! Translation between the data sources' types and the screen's types.

use borbatop_sources::{
    ContainerAction, KernelStatus, PacmanReport, Reading, Signal, UnitAction, UpdateRequest,
    UpgradeSource,
};
use borbatop_tui::{
    AurView, ContainerChoice, KernelView, PacmanView, RepoSource, ServiceAction, SignalChoice,
    StaleView, UpdatePlan,
};

/// The screen's view of a Pacman report; every failed part keeps its own message.
///
/// # Arguments
///
/// * `report` - the report built by the data sources
#[must_use]
pub fn pacman_view(report: &PacmanReport) -> PacmanView {
    PacmanView {
        upgrades: converted(report.upgrades(), Clone::clone),
        snapshot_tool: report.snapshot_tool().map(|tool| tool.program().to_owned()),
        repo_source: match report.upgrade_source() {
            UpgradeSource::Checkupdates => RepoSource::Checkupdates,
            UpgradeSource::LocalDatabase => RepoSource::LocalDatabase,
        },
        aur: report
            .aur()
            .iter()
            .map(|found| AurView {
                helper: found.helper().to_owned(),
                upgrades: converted(found.upgrades(), Clone::clone),
            })
            .collect(),
        orphans: converted(report.orphans(), Clone::clone),
        kernel: converted(report.kernel(), kernel_view),
        pending_config: converted(report.pending_config(), |config| {
            config
                .files()
                .iter()
                .map(|path| path.display().to_string())
                .collect()
        }),
        cache: converted(report.cache_size(), |size| *size),
        stale: report
            .stale()
            .findings()
            .iter()
            .map(|finding| StaleView {
                pid: finding.pid(),
                comm: finding.comm().to_owned(),
                library: finding.library().to_owned(),
            })
            .collect(),
        stale_unreadable: report.stale().unreadable(),
    }
}

/// Converts a successful reading and turns a failed one into its message.
fn converted<T, U>(reading: &Reading<T>, convert: impl FnOnce(&T) -> U) -> Result<U, String> {
    match reading {
        Ok(value) => Ok(convert(value)),
        Err(error) => Err(error.to_string()),
    }
}

fn kernel_view(status: &KernelStatus) -> KernelView {
    match status {
        KernelStatus::UpToDate { running } => KernelView {
            running: running.clone(),
            reboot_recommended: false,
        },
        KernelStatus::RebootRecommended { running } => KernelView {
            running: running.clone(),
            reboot_recommended: true,
        },
    }
}

/// The data sources' signal for a choice made in the dialog.
///
/// # Arguments
///
/// * `choice` - signal picked on screen
#[must_use]
pub const fn signal_of(choice: SignalChoice) -> Signal {
    match choice {
        SignalChoice::Terminate => Signal::Terminate,
        SignalChoice::Hangup => Signal::Hangup,
        SignalChoice::Kill => Signal::Kill,
    }
}

/// The data sources' unit action for a choice made in the service dialog.
///
/// # Arguments
///
/// * `action` - action picked on screen
#[must_use]
pub const fn unit_action_of(action: ServiceAction) -> UnitAction {
    match action {
        ServiceAction::Restart => UnitAction::Restart,
        ServiceAction::Stop => UnitAction::Stop,
        ServiceAction::Start => UnitAction::Start,
    }
}

/// The data sources' update request for what was chosen in the update dialog.
///
/// # Arguments
///
/// * `plan` - what the person chose to update
#[must_use]
pub fn update_request_of(plan: &UpdatePlan) -> UpdateRequest {
    match plan {
        UpdatePlan::Repositories => UpdateRequest::Repositories,
        UpdatePlan::AurAll { helper } => UpdateRequest::AurAll {
            helper: helper.clone(),
        },
        UpdatePlan::AurPackages { helper, packages } => UpdateRequest::AurPackages {
            helper: helper.clone(),
            packages: packages.clone(),
        },
        UpdatePlan::Snapshot { tool } => UpdateRequest::Snapshot { tool: tool.clone() },
        UpdatePlan::RepositoriesAfterSnapshot { tool } => {
            UpdateRequest::RepositoriesAfterSnapshot { tool: tool.clone() }
        }
        UpdatePlan::MergeConfigs => UpdateRequest::MergeConfigs,
        UpdatePlan::RestartUnits { units } => UpdateRequest::RestartUnits {
            units: units.clone(),
        },
    }
}

/// The data sources' container action for a choice made in the container dialog.
///
/// # Arguments
///
/// * `choice` - action picked on screen
#[must_use]
pub const fn container_action_of(choice: ContainerChoice) -> ContainerAction {
    match choice {
        ContainerChoice::Restart => ContainerAction::Restart,
        ContainerChoice::Stop => ContainerAction::Stop,
        ContainerChoice::Start => ContainerAction::Start,
        ContainerChoice::Remove => ContainerAction::Remove,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_update_plan_maps_to_the_matching_request() {
        assert_eq!(
            update_request_of(&UpdatePlan::Repositories),
            UpdateRequest::Repositories
        );
        assert_eq!(
            update_request_of(&UpdatePlan::AurPackages {
                helper: "yay".to_owned(),
                packages: vec!["a".to_owned()],
            }),
            UpdateRequest::AurPackages {
                helper: "yay".to_owned(),
                packages: vec!["a".to_owned()],
            }
        );
    }

    #[test]
    fn every_container_choice_maps_to_the_matching_verb() {
        let verbs: Vec<&str> = ContainerChoice::ALL
            .iter()
            .map(|choice| container_action_of(*choice).verb())
            .collect();
        assert_eq!(verbs, ["restart", "stop", "start", "rm"]);
    }

    #[test]
    fn every_service_choice_maps_to_the_matching_verb() {
        for action in ServiceAction::ALL {
            assert_eq!(unit_action_of(action).verb(), action.name());
        }
    }

    #[test]
    fn every_dialog_choice_maps_to_the_matching_signal() {
        for choice in SignalChoice::ALL {
            assert_eq!(signal_of(choice).name(), choice.name());
        }
    }

    #[test]
    fn a_real_report_converts_without_losing_parts() {
        let report = borbatop_sources::collect_pacman_report(&[]);
        let view = pacman_view(&report);
        assert_eq!(view.kernel.is_ok(), report.kernel().is_ok());
        assert_eq!(view.upgrades.is_ok(), report.upgrades().is_ok());
        assert_eq!(view.aur.len(), report.aur().len());
        assert_eq!(view.cache.is_ok(), report.cache_size().is_ok());
    }
}
