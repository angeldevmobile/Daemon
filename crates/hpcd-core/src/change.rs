use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{CpuSet, Risk};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Change {
    IrqAffinity { irq: u32, cpus: CpuSet },
    CpuGovernor { cpu: u32, governor: String },
    ThreadAffinity { tid: u32, cpus: CpuSet },
    CpuDmaLatency { us: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub rule_id: String,
    pub summary: String,
    pub changes: Vec<Change>,
    pub risk: Risk,
    pub reboot: bool,
    pub conflicts: Vec<String>,
    pub note: Option<String>,
}

impl fmt::Display for Change {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Change::IrqAffinity { irq, cpus } => write!(f, "IRQ {irq} → cpus {cpus}"),
            Change::CpuGovernor { cpu, governor } => write!(f, "cpu {cpu} governor → {governor}"),
            Change::ThreadAffinity { tid, cpus } => write!(f, "thread {tid} → cpus {cpus}"),
            Change::CpuDmaLatency { us } => write!(f, "hold /dev/cpu_dma_latency at {us}µs"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_each_change() {
        let cpus = CpuSet::parse("0-1").unwrap();
        assert_eq!(
            Change::IrqAffinity {
                irq: 142,
                cpus: cpus.clone()
            }
            .to_string(),
            "IRQ 142 → cpus 0-1"
        );
        assert_eq!(
            Change::ThreadAffinity { tid: 4823, cpus }.to_string(),
            "thread 4823 → cpus 0-1"
        );
        assert_eq!(
            Change::CpuDmaLatency { us: 0 }.to_string(),
            "hold /dev/cpu_dma_latency at 0µs"
        );
    }
}
