//! Cost of the hot parsers: run with `cargo bench -p borbatop-core`.
#![allow(
    missing_docs,
    reason = "criterion's macros generate undocumented items"
)]

use std::fmt::Write;
use std::hint::black_box;

use borbatop_core::{
    Pid, order_as_tree, parse_cpu_stat, parse_meminfo, parse_process_stat, parse_systemctl_units,
};
use criterion::{Criterion, criterion_group, criterion_main};

const STAT_LINE: &str = "5120 (rustc) R 2051 5120 2051 34817 5120 4194560 98765 0 12 0 6420 311 0 0 20 0 12 0 4012345 1030000000 251000 18446744073709551615 1 1 0 0 0 0 0 4096 17 3 0 0 0 0 0 0 0 0 0 0 0 0 0";
const PROCESS_COUNT: u32 = 400;
const CORE_COUNT: u32 = 12;
const UNIT_COUNT: u32 = 120;

fn cpu_stat_text() -> String {
    let mut text = String::from("cpu  4705 150 3407 8750 120 30 50 0 0 0\n");
    for core in 0..CORE_COUNT {
        let _ = writeln!(text, "cpu{core} 392 12 283 729 10 2 4 0 0 0");
    }
    text.push_str("intr 1234567 0 0 0\nctxt 99999\nbtime 1700000000\n");
    text
}

fn unit_text() -> String {
    let mut text = String::new();
    for unit in 0..UNIT_COUNT {
        let _ = writeln!(
            text,
            "service-{unit}.service loaded active running Description of service {unit}"
        );
    }
    text
}

fn parsers(criterion: &mut Criterion) {
    criterion.bench_function("parse_process_stat", |bench| {
        bench.iter(|| parse_process_stat(black_box(STAT_LINE)));
    });

    let cpu = cpu_stat_text();
    criterion.bench_function("parse_cpu_stat_12_cores", |bench| {
        bench.iter(|| parse_cpu_stat(black_box(&cpu)));
    });

    let meminfo = "MemTotal: 16215896 kB\nMemFree: 665168 kB\nMemAvailable: 4123708 kB\nBuffers: 16 kB\nCached: 4236676 kB\nSwapTotal: 16215036 kB\nSwapFree: 8657452 kB\n";
    criterion.bench_function("parse_meminfo", |bench| {
        bench.iter(|| parse_meminfo(black_box(meminfo)));
    });

    let units = unit_text();
    criterion.bench_function("parse_systemctl_units_120", |bench| {
        bench.iter(|| parse_systemctl_units(black_box(&units)));
    });

    let links: Vec<(Pid, Pid)> = (1..=PROCESS_COUNT)
        .map(|pid| (Pid::new(pid), Pid::new(pid / 3)))
        .collect();
    criterion.bench_function("order_as_tree_400", |bench| {
        bench.iter(|| order_as_tree(black_box(&links)));
    });
}

criterion_group!(benches, parsers);
criterion_main!(benches);
