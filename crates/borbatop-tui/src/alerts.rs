//! Alerts: conditions of the machine that deserve attention without anyone looking at a tab.

use std::fmt;

use borbatop_core::{ActiveState, Hundredths, Permille, Pressure};

use crate::model::Model;

const DEFAULT_CPU_PERMILLE: u64 = 900;
const DEFAULT_MEMORY_PERMILLE: u64 = 900;
const DEFAULT_SWAP_PERMILLE: u64 = 900;
const DEFAULT_PRESSURE_HUNDREDTHS: u32 = 2_000;
const DEFAULT_TEMPERATURE_DEGREES: i32 = 85;
const PERMILLE_PER_PERCENT: u64 = 10;

/// The levels from which a reading raises an alert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlertThresholds {
    /// Machine-wide CPU usage.
    pub cpu: Permille,
    /// Share of memory in use.
    pub memory: Permille,
    /// Share of swap in use.
    pub swap: Permille,
    /// Share of the last 10 seconds some task stalled on a resource (PSI `some`, `avg10`).
    pub pressure: Hundredths,
    /// CPU temperature, in degrees Celsius.
    pub temperature_degrees: i32,
}

impl Default for AlertThresholds {
    fn default() -> Self {
        Self {
            cpu: Permille::new(DEFAULT_CPU_PERMILLE),
            memory: Permille::new(DEFAULT_MEMORY_PERMILLE),
            swap: Permille::new(DEFAULT_SWAP_PERMILLE),
            pressure: Hundredths::new(DEFAULT_PRESSURE_HUNDREDTHS),
            temperature_degrees: DEFAULT_TEMPERATURE_DEGREES,
        }
    }
}

impl AlertThresholds {
    /// Thresholds given in whole percent (and degrees), as a configuration file states them.
    ///
    /// # Arguments
    ///
    /// * `cpu_percent` - machine-wide CPU usage
    /// * `memory_percent` - share of memory in use
    /// * `swap_percent` - share of swap in use
    /// * `pressure_percent` - PSI `some` `avg10`
    /// * `temperature_degrees` - CPU temperature in degrees Celsius
    #[must_use]
    pub fn from_percent(
        cpu_percent: u32,
        memory_percent: u32,
        swap_percent: u32,
        pressure_percent: u32,
        temperature_degrees: i32,
    ) -> Self {
        let permille = |percent: u32| Permille::new(u64::from(percent) * PERMILLE_PER_PERCENT);
        Self {
            cpu: permille(cpu_percent),
            memory: permille(memory_percent),
            swap: permille(swap_percent),
            pressure: Hundredths::new(pressure_percent.saturating_mul(100)),
            temperature_degrees,
        }
    }
}

/// What an alert is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertKind {
    /// Machine-wide CPU usage.
    Cpu,
    /// Memory in use.
    Memory,
    /// Swap in use.
    Swap,
    /// CPU pressure stall.
    CpuPressure,
    /// Memory pressure stall.
    MemoryPressure,
    /// I/O pressure stall.
    IoPressure,
    /// CPU temperature.
    Temperature,
    /// systemd units in the failed state.
    FailedUnits,
}

impl AlertKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Memory => "memory",
            Self::Swap => "swap",
            Self::CpuPressure => "cpu pressure",
            Self::MemoryPressure => "memory pressure",
            Self::IoPressure => "io pressure",
            Self::Temperature => "temperature",
            Self::FailedUnits => "failed units",
        }
    }
}

/// A condition above its threshold, with the reading that caused it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    /// What it is about.
    pub kind: AlertKind,
    detail: String,
}

impl fmt::Display for Alert {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} {}", self.kind.label(), self.detail)
    }
}

impl Alert {
    fn new(kind: AlertKind, detail: String) -> Self {
        Self { kind, detail }
    }
}

/// The alerts raised by `model`, in a fixed order.
///
/// # Arguments
///
/// * `model` - the sample to check
/// * `thresholds` - levels from which a reading counts
#[must_use]
pub fn evaluate(model: &Model, thresholds: &AlertThresholds) -> Vec<Alert> {
    let mut alerts = Vec::new();

    let cpu = model.cpu_history.iter().next_back().copied().unwrap_or(0);
    if cpu >= thresholds.cpu.get() {
        alerts.push(Alert::new(
            AlertKind::Cpu,
            format!("{}%", Permille::new(cpu)),
        ));
    }

    if let Some(memory) = &model.memory {
        let used = memory.used().permille_of(memory.total());
        if used >= thresholds.memory.get() {
            alerts.push(Alert::new(
                AlertKind::Memory,
                format!("{}%", Permille::new(used)),
            ));
        }
        let swapped = memory.swap_used().permille_of(memory.swap_total());
        if memory.swap_total().get() > 0 && swapped >= thresholds.swap.get() {
            alerts.push(Alert::new(
                AlertKind::Swap,
                format!("{}%", Permille::new(swapped)),
            ));
        }
    }

    pressure_alert(
        &mut alerts,
        AlertKind::CpuPressure,
        model.pressure.cpu,
        thresholds,
    );
    pressure_alert(
        &mut alerts,
        AlertKind::MemoryPressure,
        model.pressure.memory,
        thresholds,
    );
    pressure_alert(
        &mut alerts,
        AlertKind::IoPressure,
        model.pressure.io,
        thresholds,
    );

    if let Some(temperature) = model.temperature {
        let degrees = temperature.rounded_degrees();
        if degrees >= thresholds.temperature_degrees {
            alerts.push(Alert::new(AlertKind::Temperature, format!("{degrees}°C")));
        }
    }

    let failed = failed_units(model);
    if failed > 0 {
        alerts.push(Alert::new(AlertKind::FailedUnits, failed.to_string()));
    }
    alerts
}

fn pressure_alert(
    alerts: &mut Vec<Alert>,
    kind: AlertKind,
    pressure: Option<Pressure>,
    thresholds: &AlertThresholds,
) {
    let Some(pressure) = pressure else {
        return;
    };
    let level = pressure.some().avg10();
    if level >= thresholds.pressure {
        alerts.push(Alert::new(kind, format!("{level}%")));
    }
}

fn failed_units(model: &Model) -> usize {
    model.services.as_ref().map_or(0, |list| {
        list.units()
            .iter()
            .filter(|unit| unit.active() == ActiveState::Failed)
            .count()
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use borbatop_core::{MilliCelsius, parse_meminfo, parse_pressure, parse_systemctl_units};

    use super::*;

    const MEMINFO: &str = "MemTotal: 1000 kB\nMemFree: 0 kB\nMemAvailable: 40 kB\nSwapTotal: 1000 kB\nSwapFree: 900 kB\n";

    fn calm() -> Model {
        let mut model = Model::empty(4);
        model.cpu_history.push(100);
        model
    }

    fn texts(model: &Model) -> Vec<String> {
        evaluate(model, &AlertThresholds::default())
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    #[test]
    fn a_calm_machine_raises_nothing() {
        assert_eq!(texts(&calm()), Vec::<String>::new());
    }

    #[test]
    fn each_reading_above_its_threshold_raises_one_alert() {
        let mut model = calm();
        model.cpu_history.push(950);
        model.memory = Some(parse_meminfo(MEMINFO).unwrap());
        model.pressure.memory = Some(
            parse_pressure(
                "some avg10=31.50 avg60=1.00 avg300=0.00 total=1\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=0\n",
            )
            .unwrap(),
        );
        model.temperature = Some(MilliCelsius::new(91_000));
        model.services = Ok(parse_systemctl_units(
            "a.service loaded failed failed A\nb.service loaded failed failed B\nc.service loaded active running C\n",
        ));

        assert_eq!(
            texts(&model),
            [
                "cpu 95.0%",
                "memory 96.0%",
                "memory pressure 31.50%",
                "temperature 91°C",
                "failed units 2",
            ]
        );
    }

    #[test]
    fn swap_only_counts_when_there_is_swap() {
        let mut model = calm();
        model.memory = Some(
            parse_meminfo("MemTotal: 1000 kB\nMemFree: 900 kB\nMemAvailable: 900 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n")
                .unwrap(),
        );
        assert_eq!(texts(&model), Vec::<String>::new());

        model.memory = Some(
            parse_meminfo(
                "MemTotal: 1000 kB\nMemFree: 900 kB\nMemAvailable: 900 kB\nSwapTotal: 1000 kB\nSwapFree: 50 kB\n",
            )
            .unwrap(),
        );
        assert_eq!(texts(&model), ["swap 95.0%"]);
    }

    #[test]
    fn thresholds_come_from_percent_and_are_inclusive() {
        let thresholds = AlertThresholds::from_percent(50, 60, 70, 80, 90);
        assert_eq!(thresholds.cpu.get(), 500);
        assert_eq!(thresholds.pressure.get(), 8_000);

        let mut model = calm();
        model.cpu_history.push(500);
        assert_eq!(evaluate(&model, &thresholds).len(), 1);
        model.cpu_history.push(499);
        assert_eq!(evaluate(&model, &thresholds), Vec::<Alert>::new());
    }

    #[test]
    fn missing_readings_raise_nothing() {
        let mut model = Model::empty(4);
        model.services = Err("no systemd".to_owned());
        assert_eq!(texts(&model), Vec::<String>::new());
    }
}
