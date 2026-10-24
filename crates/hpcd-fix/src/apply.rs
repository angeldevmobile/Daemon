use std::io;

use hpcd_core::{Change, CpuSet};
use serde::{Deserialize, Serialize};

use crate::FixError;
use crate::system::{Hold, System};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Previous {
    Cpus { cpus: CpuSet },
    Governor { governor: String },
    Nothing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Applied {
    pub change: Change,
    pub previous: Previous,
}

fn irq_path(irq: u32) -> String {
    format!("/proc/irq/{irq}/smp_affinity_list")
}

fn governor_path(cpu: u32) -> String {
    format!("/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor")
}

fn io_err(change: &Change) -> impl FnOnce(io::Error) -> FixError + '_ {
    move |source| FixError::Io {
        change: change.to_string(),
        source,
    }
}

fn read_cpus(sys: &impl System, path: &str, change: &Change) -> Result<CpuSet, FixError> {
    let text = sys.read(path).map_err(io_err(change))?;
    CpuSet::parse(&text).ok_or_else(|| FixError::Rejected {
        change: change.to_string(),
        got: text.trim().to_string(),
    })
}

pub fn snapshot(sys: &impl System, change: &Change) -> Result<Applied, FixError> {
    let previous = match change {
        Change::IrqAffinity { irq, .. } => Previous::Cpus {
            cpus: read_cpus(sys, &irq_path(*irq), change)?,
        },
        Change::CpuGovernor { cpu, .. } => Previous::Governor {
            governor: sys
                .read(&governor_path(*cpu))
                .map_err(io_err(change))?
                .trim()
                .to_string(),
        },
        Change::ThreadAffinity { tid, .. } => Previous::Cpus {
            cpus: sys.affinity(*tid).map_err(io_err(change))?,
        },
        Change::CpuDmaLatency { .. } => Previous::Nothing,
    };
    Ok(Applied {
        change: change.clone(),
        previous,
    })
}

fn current(sys: &impl System, change: &Change) -> Result<String, FixError> {
    Ok(match change {
        Change::IrqAffinity { irq, .. } => read_cpus(sys, &irq_path(*irq), change)?.to_string(),
        Change::CpuGovernor { cpu, .. } => sys
            .read(&governor_path(*cpu))
            .map_err(io_err(change))?
            .trim()
            .to_string(),
        Change::ThreadAffinity { tid, .. } => {
            sys.affinity(*tid).map_err(io_err(change))?.to_string()
        }
        Change::CpuDmaLatency { .. } => String::new(),
    })
}

fn wanted(change: &Change) -> String {
    match change {
        Change::IrqAffinity { cpus, .. } | Change::ThreadAffinity { cpus, .. } => cpus.to_string(),
        Change::CpuGovernor { governor, .. } => governor.clone(),
        Change::CpuDmaLatency { .. } => String::new(),
    }
}

pub fn write(sys: &impl System, change: &Change) -> Result<Option<Hold>, FixError> {
    let hold = match change {
        Change::IrqAffinity { irq, cpus } => {
            sys.write(&irq_path(*irq), &cpus.to_string())
                .map_err(io_err(change))?;
            None
        }
        Change::CpuGovernor { cpu, governor } => {
            sys.write(&governor_path(*cpu), governor)
                .map_err(io_err(change))?;
            None
        }
        Change::ThreadAffinity { tid, cpus } => {
            sys.set_affinity(*tid, cpus).map_err(io_err(change))?;
            None
        }
        Change::CpuDmaLatency { us } => Some(sys.hold_dma_latency(*us).map_err(io_err(change))?),
    };
    let got = current(sys, change)?;
    if got != wanted(change) {
        return Err(FixError::Rejected {
            change: change.to_string(),
            got,
        });
    }
    Ok(hold)
}

fn gone(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::NotFound || e.raw_os_error() == Some(3)
}

pub fn restore(sys: &impl System, applied: &Applied) -> Result<(), FixError> {
    let change = &applied.change;
    let result = match (change, &applied.previous) {
        (Change::IrqAffinity { irq, .. }, Previous::Cpus { cpus }) => {
            sys.write(&irq_path(*irq), &cpus.to_string())
        }
        (Change::CpuGovernor { cpu, .. }, Previous::Governor { governor }) => {
            sys.write(&governor_path(*cpu), governor)
        }
        (Change::ThreadAffinity { tid, .. }, Previous::Cpus { cpus }) => {
            sys.set_affinity(*tid, cpus)
        }
        (Change::CpuDmaLatency { .. }, _) => Ok(()),
        _ => {
            return Err(FixError::Rejected {
                change: change.to_string(),
                got: "a snapshot of another kind".into(),
            });
        }
    };
    match result {
        Err(e) if gone(&e) => Ok(()),
        other => other.map_err(io_err(change)),
    }
}
