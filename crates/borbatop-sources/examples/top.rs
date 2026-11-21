//! Samples the machine twice and prints the busiest processes:
//! `cargo run -p borbatop-sources --example top`.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::error::Error;
use std::thread;
use std::time::Duration;

use borbatop_core::{
    CpuTicks, Permille, Pid, ProcessId, SamplePair, busy_permille, cpu_permille, elapsed_per_core,
};
use borbatop_sources::{ProcessStatics, SystemCollector};

const SAMPLE_INTERVAL: Duration = Duration::from_millis(1_000);
const TOP_COUNT: usize = 10;

fn main() -> Result<(), Box<dyn Error>> {
    let mut collector = SystemCollector::system()?;

    let first = collector.collect()?;
    thread::sleep(SAMPLE_INTERVAL);
    let second = collector.collect()?;

    let machine = SamplePair::new(
        first.processes().cpu().total(),
        second.processes().cpu().total(),
    );
    let core_count = second.processes().cpu().cores().len();
    let elapsed = elapsed_per_core(machine, core_count)?;
    println!(
        "machine: {}% busy over {core_count} cores | {} processes, {} vanished, {} failed",
        busy_permille(machine)?,
        second.processes().processes().len(),
        second.processes().vanished(),
        second.processes().failures().len(),
    );
    if let Ok(memory) = second.memory() {
        println!("memory: {} used of {}", memory.used(), memory.total());
    }
    if let Ok(temperature) = second.temperature() {
        println!("temperature: {temperature}");
    }

    // Keyed by PID plus start time: a reused PID is a different process, not a CPU spike.
    let previous: HashMap<ProcessId, CpuTicks> = first
        .processes()
        .processes()
        .iter()
        .map(|record| (record.sample().id(), record.sample().cpu_ticks()))
        .collect();

    let mut usage: Vec<(Permille, Pid, &ProcessStatics)> = Vec::new();
    for record in second.processes().processes() {
        let sample = record.sample();
        let Some(&before) = previous.get(&sample.id()) else {
            continue;
        };
        let window = SamplePair::new(before, sample.cpu_ticks());
        if let Ok(permille) = cpu_permille(window, elapsed) {
            usage.push((permille, sample.pid(), record.statics()));
        }
    }
    usage.sort_unstable_by_key(|entry| Reverse(entry.0));

    println!(
        "{:>8} {:>7} {:<10} {:<16} COMMAND",
        "PID", "CPU%", "USER", "PACKAGE"
    );
    for (permille, pid, statics) in usage.into_iter().take(TOP_COUNT) {
        println!(
            "{pid:>8} {permille:>7} {:<10} {:<16} {}",
            statics.user(),
            statics.package(),
            statics.command().chars().take(60).collect::<String>(),
        );
    }

    Ok(())
}
