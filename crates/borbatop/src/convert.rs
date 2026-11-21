//! Translation between the data sources' types and the screen's types.

use borbatop_sources::{KernelStatus, PacmanReport, Reading, Signal};
use borbatop_tui::{KernelView, PacmanView, SignalChoice, StaleView};

/// The screen's view of a Pacman report; every failed part keeps its own message.
///
/// # Arguments
///
/// * `report` - the report built by the data sources
#[must_use]
pub fn pacman_view(report: &PacmanReport) -> PacmanView {
    PacmanView {
        upgrades: converted(report.upgrades(), Clone::clone),
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(view.cache.is_ok(), report.cache_size().is_ok());
    }
}
