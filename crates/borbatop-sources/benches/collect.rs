//! Cost of one full read of the machine: run with `cargo bench -p borbatop-sources`.
#![allow(
    missing_docs,
    reason = "criterion's macros generate undocumented items"
)]
#![allow(
    clippy::expect_used,
    reason = "a benchmark that cannot read the machine must stop"
)]

use borbatop_sources::{ProcCollector, SystemCollector};
use criterion::{Criterion, criterion_group, criterion_main};

fn collectors(criterion: &mut Criterion) {
    let mut processes = ProcCollector::system().expect("procfs and passwd must be readable");
    // The first call fills the per-process cache; measure the steady state.
    processes.collect().expect("first read");
    criterion.bench_function("proc_collect_steady_state", |bench| {
        bench.iter(|| {
            processes
                .collect()
                .map(|snapshot| snapshot.processes().len())
        });
    });

    let mut system = SystemCollector::system().expect("system must be readable");
    system.collect().expect("first read");
    criterion.bench_function("system_collect_steady_state", |bench| {
        bench.iter(|| {
            system
                .collect()
                .map(|snapshot| snapshot.processes().processes().len())
        });
    });
}

criterion_group!(benches, collectors);
criterion_main!(benches);
