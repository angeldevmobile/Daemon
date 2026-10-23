use hpcd_probe::Observation;

use crate::{Hit, Rule, checks};

pub type CheckFn = fn(&Rule, &Observation) -> Result<Option<Hit>, String>;

pub struct Check {
    pub name: &'static str,
    pub description: &'static str,
    pub run: CheckFn,
}

pub struct Action {
    pub name: &'static str,
    pub description: &'static str,
    pub revert: &'static str,
}

pub const CHECKS: &[Check] = &[
    Check {
        name: "irq_affinity_overlaps",
        description: "IRQs whose affinity includes cores running hot threads",
        run: checks::irq_affinity_overlaps,
    },
    Check {
        name: "cpuidle_deep_states",
        description: "Enabled C-states whose exit latency exceeds a limit on hot cores",
        run: checks::cpuidle_deep_states,
    },
    Check {
        name: "cpufreq_governor",
        description: "Frequency governor on hot cores is one of a given list",
        run: checks::cpufreq_governor,
    },
];

pub const ACTIONS: &[Action] = &[
    Action {
        name: "set_irq_affinity",
        description: "Set the CPU affinity of IRQs",
        revert: "restore each IRQ's previous smp_affinity_list",
    },
    Action {
        name: "hold_cpu_dma_latency",
        description: "Hold a PM QoS latency request on /dev/cpu_dma_latency",
        revert: "close /dev/cpu_dma_latency, releasing the request",
    },
    Action {
        name: "set_cpufreq_governor",
        description: "Set the scaling governor on selected cores",
        revert: "restore each core's previous scaling_governor",
    },
];

pub fn check(name: &str) -> Option<&'static Check> {
    CHECKS.iter().find(|e| e.name == name)
}

pub fn action(name: &str) -> Option<&'static Action> {
    ACTIONS.iter().find(|e| e.name == name)
}
