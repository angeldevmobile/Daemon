use hpcd_probe::Observation;

use crate::plan::{self, Draft};
use crate::{Fix, Hit, Rule, checks};

pub type CheckFn = fn(&Rule, &Observation) -> Result<Option<Hit>, String>;
pub type PlanFn = fn(&Rule, &Fix, &Observation) -> Result<Draft, String>;

pub struct Check {
    pub name: &'static str,
    pub description: &'static str,
    pub run: CheckFn,
}

pub struct Action {
    pub name: &'static str,
    pub description: &'static str,
    pub revert: &'static str,
    pub plan: PlanFn,
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
    Check {
        name: "process_running",
        description: "A process with one of the given names is running",
        run: checks::process_running,
    },
    Check {
        name: "tuned_profile",
        description: "tuned is running with an active profile",
        run: checks::tuned_profile,
    },
    Check {
        name: "cpu_isolation",
        description: "Hot cores are not in the isolated set (isolcpus)",
        run: checks::cpu_isolation,
    },
    Check {
        name: "smt_siblings",
        description: "SMT is active and hot cores share a physical core",
        run: checks::smt_siblings,
    },
    Check {
        name: "thread_migration",
        description: "Hot threads migrate between cores more often than a given rate",
        run: checks::thread_migration,
    },
    Check {
        name: "smt_sibling_busy",
        description: "The SMT sibling of a hot core is busy with other work",
        run: checks::smt_sibling_busy,
    },
    Check {
        name: "numa_remote_memory",
        description: "A large share of the target's memory is on a node remote to its hot threads",
        run: checks::numa_remote_memory,
    },
    Check {
        name: "sub_numa_clustering",
        description: "More NUMA nodes than CPU sockets (SNC or NPS enabled in firmware)",
        run: checks::sub_numa_clustering,
    },
    Check {
        name: "smi_count",
        description: "System Management Interrupts above a rate (MSR_SMI_COUNT, Intel, needs root)",
        run: checks::smi_count,
    },
];

pub const ACTIONS: &[Action] = &[
    Action {
        name: "set_irq_affinity",
        description: "Set the CPU affinity of IRQs",
        revert: "restore each IRQ's previous smp_affinity_list",
        plan: plan::set_irq_affinity,
    },
    Action {
        name: "hold_cpu_dma_latency",
        description: "Hold a PM QoS latency request on /dev/cpu_dma_latency",
        revert: "close /dev/cpu_dma_latency, releasing the request",
        plan: plan::hold_cpu_dma_latency,
    },
    Action {
        name: "set_cpufreq_governor",
        description: "Set the scaling governor on selected cores",
        revert: "restore each core's previous scaling_governor",
        plan: plan::set_cpufreq_governor,
    },
    Action {
        name: "propose_kernel_cmdline",
        description: "Propose kernel command line parameters; never applied",
        revert: "nothing is applied; remove the parameters and reboot to undo",
        plan: plan::propose_kernel_cmdline,
    },
    Action {
        name: "pin_thread",
        description: "Pin a thread to one core with sched_setaffinity",
        revert: "restore the thread's previous CPU affinity",
        plan: plan::pin_thread,
    },
    Action {
        name: "move_tasks_off_cpu",
        description: "Move movable tasks off a core by narrowing their affinity",
        revert: "restore each moved task's previous CPU affinity",
        plan: plan::move_tasks_off_cpu,
    },
];

pub fn check(name: &str) -> Option<&'static Check> {
    CHECKS.iter().find(|e| e.name == name)
}

pub fn action(name: &str) -> Option<&'static Action> {
    ACTIONS.iter().find(|e| e.name == name)
}
