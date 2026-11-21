//! Turns raw system snapshots into the screen's [`Model`]: usage from counter deltas,
//! throughput from byte deltas and bounded histories for sparklines.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use borbatop_core::{
    Bytes, CpuSnapshot, CpuTicks, ElapsedTicks, History, PageSize, Permille, ProcessId, SamplePair,
    UnitList, busy_permille, bytes_per_second, cpu_permille, elapsed_per_core,
};
use borbatop_sources::{ProcessRecord, SystemSnapshot};
use borbatop_tui::{DiskRow, Model, MountRow, NetRow, PressureView, ProcessRow};

const LOOPBACK: &str = "lo";

/// What the previous sample looked like, to compute deltas against.
struct Previous {
    taken_at: Instant,
    cpu: CpuSnapshot,
    ticks: HashMap<ProcessId, CpuTicks>,
    network: HashMap<String, (Bytes, Bytes)>,
    disks: HashMap<String, (Bytes, Bytes)>,
}

/// Builds one [`Model`] per sample, remembering what the next one needs.
pub struct ModelBuilder {
    history_len: usize,
    page_size: PageSize,
    kernel_release: String,
    previous: Option<Previous>,
    cpu_history: History<u64>,
    network_history: HashMap<String, (History<u64>, History<u64>)>,
    services: Result<UnitList, String>,
}

impl ModelBuilder {
    /// A builder for a machine with the given page size.
    ///
    /// # Arguments
    ///
    /// * `history_len` - samples kept for sparklines
    /// * `page_size` - memory page size, to turn resident pages into bytes
    /// * `kernel_release` - running kernel release, shown in the header
    #[must_use]
    pub fn new(history_len: usize, page_size: PageSize, kernel_release: String) -> Self {
        Self {
            history_len,
            page_size,
            kernel_release,
            previous: None,
            cpu_history: History::new(history_len),
            network_history: HashMap::new(),
            services: Err("systemd units not loaded yet".to_owned()),
        }
    }

    /// Replaces the unit list shown in the Services tab.
    ///
    /// # Arguments
    ///
    /// * `services` - the units, or why they could not be listed
    pub fn set_services(&mut self, services: Result<UnitList, String>) {
        self.services = services;
    }

    /// Builds the model of `snapshot`.
    ///
    /// The first call has nothing to compare with, so every usage and rate reads zero.
    ///
    /// # Arguments
    ///
    /// * `snapshot` - the newest read of the machine
    pub fn build(&mut self, snapshot: &SystemSnapshot) -> Model {
        let processes = snapshot.processes();
        let cpu = processes.cpu();
        let elapsed_wall = self.previous.as_ref().map(|previous| {
            snapshot
                .taken_at()
                .saturating_duration_since(previous.taken_at)
        });

        let cores = self.core_usage(cpu);
        let total = self.total_usage(cpu);
        self.cpu_history.push(total.get());
        let rows = self.process_rows(processes.processes(), cpu);

        let network = self.network_rows(snapshot, elapsed_wall);
        let disks = self.disk_rows(snapshot, elapsed_wall);
        self.remember(snapshot);

        Model {
            processes: rows,
            vanished: processes.vanished(),
            failed: processes.failures().len(),
            cores,
            cpu_history: self.cpu_history.clone(),
            memory: snapshot.memory().as_ref().ok().copied(),
            load: snapshot.load().as_ref().ok().copied(),
            uptime: snapshot.uptime().as_ref().ok().copied(),
            temperature: snapshot.temperature().as_ref().ok().copied(),
            pressure: PressureView {
                cpu: snapshot.cpu_pressure().as_ref().ok().copied(),
                memory: snapshot.memory_pressure().as_ref().ok().copied(),
                io: snapshot.io_pressure().as_ref().ok().copied(),
            },
            kernel_release: self.kernel_release.clone(),
            services: self.services.clone(),
            network,
            disks,
            mounts: mount_rows(snapshot),
        }
    }

    fn core_usage(&self, cpu: &CpuSnapshot) -> Vec<Permille> {
        let Some(previous) = &self.previous else {
            return vec![Permille::new(0); cpu.cores().len()];
        };
        cpu.cores()
            .iter()
            .enumerate()
            .map(|(index, &current)| {
                previous
                    .cpu
                    .cores()
                    .get(index)
                    .and_then(|&before| busy_permille(SamplePair::new(before, current)).ok())
                    .unwrap_or(Permille::new(0))
            })
            .collect()
    }

    fn total_usage(&self, cpu: &CpuSnapshot) -> Permille {
        self.previous
            .as_ref()
            .and_then(|previous| {
                busy_permille(SamplePair::new(previous.cpu.total(), cpu.total())).ok()
            })
            .unwrap_or(Permille::new(0))
    }

    fn process_rows(&self, records: &[ProcessRecord], cpu: &CpuSnapshot) -> Vec<ProcessRow> {
        let elapsed = self.previous.as_ref().and_then(|previous| {
            let window = SamplePair::new(previous.cpu.total(), cpu.total());
            elapsed_per_core(window, cpu.cores().len()).ok()
        });

        records
            .iter()
            .map(|record| {
                let sample = record.sample();
                let usage = self.process_usage(sample.id(), sample.cpu_ticks(), elapsed);
                let statics = record.statics();
                ProcessRow {
                    pid: sample.pid(),
                    start_time: sample.id().start_time(),
                    parent: sample.parent(),
                    user: statics.user().to_owned(),
                    state: sample.state(),
                    cpu: usage,
                    memory: self
                        .page_size
                        .bytes_in(sample.resident_pages())
                        .unwrap_or_default(),
                    threads: sample.threads(),
                    unit: statics.unit().to_owned(),
                    package: statics.package().to_owned(),
                    command: statics.command().to_owned(),
                }
            })
            .collect()
    }

    fn process_usage(
        &self,
        id: ProcessId,
        ticks: CpuTicks,
        elapsed: Option<ElapsedTicks>,
    ) -> Permille {
        // Keyed by PID plus start time: a reused PID is a new process, never a CPU spike.
        let before = self
            .previous
            .as_ref()
            .and_then(|previous| previous.ticks.get(&id));
        match (before, elapsed) {
            (Some(&before), Some(elapsed)) => {
                cpu_permille(SamplePair::new(before, ticks), elapsed).unwrap_or(Permille::new(0))
            }
            _ => Permille::new(0),
        }
    }

    fn network_rows(
        &mut self,
        snapshot: &SystemSnapshot,
        elapsed: Option<Duration>,
    ) -> Vec<NetRow> {
        let Ok(interfaces) = snapshot.network() else {
            return Vec::new();
        };
        let history_len = self.history_len;

        interfaces
            .iter()
            .filter(|interface| interface.interface() != LOOPBACK)
            .map(|interface| {
                let before = self
                    .previous
                    .as_ref()
                    .and_then(|previous| previous.network.get(interface.interface()));
                let received = rate(before.map(|pair| pair.0), interface.received(), elapsed);
                let sent = rate(before.map(|pair| pair.1), interface.sent(), elapsed);

                let (received_history, sent_history) = self
                    .network_history
                    .entry(interface.interface().to_owned())
                    .or_insert_with(|| (History::new(history_len), History::new(history_len)));
                received_history.push(received.map_or(0, borbatop_core::BytesPerSecond::get));
                sent_history.push(sent.map_or(0, borbatop_core::BytesPerSecond::get));

                NetRow {
                    name: interface.interface().to_owned(),
                    received,
                    sent,
                    received_history: received_history.clone(),
                    sent_history: sent_history.clone(),
                }
            })
            .collect()
    }

    fn disk_rows(&self, snapshot: &SystemSnapshot, elapsed: Option<Duration>) -> Vec<DiskRow> {
        let Ok(disks) = snapshot.disks() else {
            return Vec::new();
        };
        disks
            .iter()
            .map(|disk| {
                let before = self
                    .previous
                    .as_ref()
                    .and_then(|previous| previous.disks.get(disk.device()));
                DiskRow {
                    name: disk.device().to_owned(),
                    read: rate(before.map(|pair| pair.0), disk.read(), elapsed),
                    written: rate(before.map(|pair| pair.1), disk.written(), elapsed),
                }
            })
            .collect()
    }

    fn remember(&mut self, snapshot: &SystemSnapshot) {
        let processes = snapshot.processes();
        let counters =
            |names: Option<Vec<(String, Bytes, Bytes)>>| -> HashMap<String, (Bytes, Bytes)> {
                names
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(name, first, second)| (name, (first, second)))
                    .collect()
            };
        let network = counters(snapshot.network().as_ref().ok().map(|interfaces| {
            interfaces
                .iter()
                .map(|net| (net.interface().to_owned(), net.received(), net.sent()))
                .collect()
        }));
        let disks = counters(snapshot.disks().as_ref().ok().map(|disks| {
            disks
                .iter()
                .map(|disk| (disk.device().to_owned(), disk.read(), disk.written()))
                .collect()
        }));

        self.previous = Some(Previous {
            taken_at: snapshot.taken_at(),
            cpu: processes.cpu().clone(),
            ticks: processes
                .processes()
                .iter()
                .map(|record| (record.sample().id(), record.sample().cpu_ticks()))
                .collect(),
            network,
            disks,
        });
    }
}

fn rate(
    before: Option<Bytes>,
    now: Bytes,
    elapsed: Option<Duration>,
) -> Option<borbatop_core::BytesPerSecond> {
    bytes_per_second(before?, now, elapsed?)
}

fn mount_rows(snapshot: &SystemSnapshot) -> Vec<MountRow> {
    let Ok(report) = snapshot.mounts() else {
        return Vec::new();
    };
    report
        .usages()
        .iter()
        .map(|usage| MountRow {
            mount_point: usage.mount().mount_point().to_owned(),
            fs_type: usage.mount().fs_type().to_owned(),
            total: usage.total(),
            used: usage.used(),
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::thread;

    use borbatop_sources::SystemCollector;

    use super::*;

    fn builder() -> ModelBuilder {
        ModelBuilder::new(
            8,
            SystemCollector::page_size().unwrap(),
            "7.2.11-test".to_owned(),
        )
    }

    #[test]
    fn first_sample_reads_zero_usage_but_has_everything_else() {
        let mut collector = SystemCollector::system().unwrap();
        let mut builder = builder();

        let model = builder.build(&collector.collect().unwrap());

        assert_ne!(model.processes.len(), 0);
        assert!(model.cores.iter().all(|core| core.get() == 0));
        assert!(model.processes.iter().all(|row| row.cpu.get() == 0));
        assert!(model.memory.is_some());
        assert_eq!(model.kernel_release, "7.2.11-test");
    }

    #[test]
    fn second_sample_measures_a_busy_loop_and_bounds_histories() {
        let mut collector = SystemCollector::system().unwrap();
        let mut builder = builder();
        let own = std::process::id();
        builder.build(&collector.collect().unwrap());

        let spinner = thread::spawn(|| {
            let end = Instant::now() + Duration::from_millis(600);
            let mut counter = 0_u64;
            while Instant::now() < end {
                counter = std::hint::black_box(counter.wrapping_add(1));
            }
            counter
        });
        thread::sleep(Duration::from_millis(300));
        let model = builder.build(&collector.collect().unwrap());
        spinner.join().unwrap();

        let me = model
            .processes
            .iter()
            .find(|row| row.pid.get() == own)
            .unwrap();
        assert!(
            me.cpu.get() > 100,
            "busy loop should show above 10%: {}",
            me.cpu
        );
        assert!(me.memory.get() > 0);
        assert!(model.cores.iter().any(|core| core.get() > 0));
        assert!(model.cpu_history.len() <= 8);
        assert!(model.cpu_history.latest().copied().unwrap_or(0) <= 1_000);
        assert!(model.cpu_history.len() >= 2);
    }

    #[test]
    fn services_are_passed_through() {
        let mut collector = SystemCollector::system().unwrap();
        let mut builder = builder();
        builder.set_services(Err("no systemd".to_owned()));
        let model = builder.build(&collector.collect().unwrap());
        assert_eq!(
            model.services.as_ref().err().map(String::as_str),
            Some("no systemd")
        );
    }

    #[test]
    fn rates_need_both_a_previous_reading_and_elapsed_time() {
        let second = Some(Duration::from_secs(1));
        assert_eq!(
            rate(Some(Bytes::new(0)), Bytes::new(100), second)
                .map(borbatop_core::BytesPerSecond::get),
            Some(100)
        );
        assert!(rate(None, Bytes::new(100), second).is_none());
        assert!(rate(Some(Bytes::new(0)), Bytes::new(100), None).is_none());
        assert!(rate(Some(Bytes::new(500)), Bytes::new(100), second).is_none());
    }
}
