//! Samples `/proc` twice and prints the busiest processes: `cargo run -p borbatop-sources --example top`.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::error::Error;
use std::thread;
use std::time::Duration;

use borbatop_core::{
    CpuTicks, Permille, Pid, ProcessId, SamplePair, busy_permille, cpu_permille, elapsed_per_core,
};
use borbatop_sources::ProcCollector;

const SAMPLE_INTERVAL: Duration = Duration::from_millis(1_000);
const TOP_COUNT: usize = 10;

fn main() -> Result<(), Box<dyn Error>> {
    let mut collector = ProcCollector::system();

    let first = collector.collect()?;
    thread::sleep(SAMPLE_INTERVAL);
    let second = collector.collect()?;

    let machine = SamplePair::new(first.cpu().total(), second.cpu().total());
    let core_count = second.cpu().cores().len();
    let elapsed = elapsed_per_core(machine, core_count)?;
    println!(
        "machine: {}% busy over {core_count} cores | {} processes, {} vanished, {} failed",
        busy_permille(machine)?,
        second.processes().len(),
        second.vanished(),
        second.failures().len(),
    );

    // Keyed by PID plus start time: a reused PID is a different process, not a CPU spike.
    let previous: HashMap<ProcessId, CpuTicks> = first
        .processes()
        .iter()
        .map(|sample| (sample.id(), sample.cpu_ticks()))
        .collect();

    let mut usage: Vec<(Permille, Pid, &str)> = Vec::with_capacity(second.processes().len());
    for sample in second.processes() {
        let Some(&before) = previous.get(&sample.id()) else {
            continue;
        };
        let window = SamplePair::new(before, sample.cpu_ticks());
        if let Ok(permille) = cpu_permille(window, elapsed) {
            usage.push((permille, sample.pid(), sample.comm()));
        }
    }
    usage.sort_unstable_by_key(|entry| Reverse(entry.0));

    println!("{:>8} {:>7}  COMMAND", "PID", "CPU%");
    for (permille, pid, comm) in usage.into_iter().take(TOP_COUNT) {
        println!("{pid:>8} {permille:>7}  {comm}");
    }

    Ok(())
}
