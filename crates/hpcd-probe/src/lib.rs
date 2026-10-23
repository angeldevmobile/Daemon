mod capture;
mod cpu;
mod cpuset;
mod interrupts;
mod machine;
mod numa;
mod observe;
mod source;
mod tasks;

use std::path::PathBuf;

pub use capture::{Capture, capture};
pub use cpu::{CpuInfo, IdleState};
pub use cpuset::CpuSet;
pub use interrupts::IrqLine;
pub use machine::Machine;
pub use observe::{HOT_CPU_SHARE, Irq, Observation, Thread, observe};
pub use source::{Source, SysRoot};
pub use tasks::{Process, TaskStat};

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
