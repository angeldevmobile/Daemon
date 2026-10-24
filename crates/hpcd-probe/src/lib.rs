mod capture;
mod cpu;
mod interrupts;
mod load;
mod machine;
mod numa;
mod observe;
mod source;
mod tasks;

use std::path::PathBuf;

pub use capture::{Capture, capture};
pub use cpu::{Cache, CpuInfo, IdleState};
pub use hpcd_core::CpuSet;
pub use interrupts::IrqLine;
pub use load::{Load, LoadCounters, read_load};
pub use machine::Machine;
pub use observe::{HOT_CPU_SHARE, Irq, MSR_SMI_COUNT, Observation, Thread, observe, observe_host};
pub use source::{Source, SysRoot};
pub use tasks::{Process, TaskStat, resolve_pids};

#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("{0}")]
    Target(String),
}
