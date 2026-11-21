//! Pure sampling core: no I/O, no clock.
//!
//! Everything here takes text or numbers in and returns values or typed errors out, so it can
//! be tested without a Linux machine.

mod cpu;
mod process;

pub use cpu::{
    CpuSnapshot, CpuStatError, CpuTicks, CpuTimes, ElapsedTicks, Permille, SampleError, SamplePair,
    busy_permille, cpu_permille, elapsed_per_core, parse_cpu_stat,
};
pub use process::{
    Pages, Pid, ProcessId, ProcessSample, ProcessState, StartTime, StatField, StatParseError,
    ThreadCount, UnknownStateCode, parse_process_stat,
};
