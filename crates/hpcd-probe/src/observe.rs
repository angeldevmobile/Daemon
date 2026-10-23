use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use hpcd_core::Target;

use crate::cpu::{self, CpuInfo};
use crate::interrupts::{self, IrqLine};
use crate::tasks::{self, Process, TaskStat};
use crate::{CpuSet, ProbeError, Source, SysRoot};

pub const HOT_CPU_SHARE: f64 = 0.05;
const USER_HZ: f64 = 100.0;

#[derive(Debug, Clone)]
pub struct Thread {
    pub pid: u32,
    pub tid: u32,
    pub comm: String,
    pub cpu_share: f64,
    pub core: u32,
}

#[derive(Debug, Clone)]
pub struct Irq {
    pub label: String,
    pub name: String,
    pub description: String,
    pub rates: BTreeMap<u32, f64>,
    pub affinity: Option<CpuSet>,
}

impl Irq {
    pub fn number(&self) -> Option<u32> {
        self.label.parse().ok()
    }

    pub fn rate_on(&self, cpu: u32) -> f64 {
        self.rates.get(&cpu).copied().unwrap_or(0.0)
    }
}

#[derive(Debug, Clone)]
pub struct Observation {
    pub interval: Duration,
    pub pids: Vec<u32>,
    pub threads: Vec<Thread>,
    pub irqs: Vec<Irq>,
    pub cpus: BTreeMap<u32, CpuInfo>,
    pub processes: Vec<Process>,
    pub tuned_profile: Option<String>,
    pub isolated: Option<CpuSet>,
    pub smt_active: Option<bool>,
}

impl Observation {
    pub fn hot_threads(&self) -> impl Iterator<Item = &Thread> {
        self.threads.iter().filter(|t| t.cpu_share >= HOT_CPU_SHARE)
    }

    pub fn hot_cores(&self) -> CpuSet {
        self.hot_threads().map(|t| t.core).collect()
    }
}

struct Sample {
    irqs: Vec<IrqLine>,
    threads: Vec<(u32, TaskStat)>,
}

fn sample(root: &SysRoot, pids: &[u32]) -> Result<Sample, ProbeError> {
    let irqs =
        interrupts::parse(&root.read("/proc/interrupts")?).ok_or_else(|| ProbeError::Parse {
            path: root.resolve("/proc/interrupts"),
            message: "unexpected format".into(),
        })?;
    let mut threads = Vec::new();
    for &pid in pids {
        threads.extend(
            tasks::read_threads(root, pid)?
                .into_iter()
                .map(|t| (pid, t)),
        );
    }
    Ok(Sample { irqs, threads })
}

pub fn observe(source: &Source, target: &Target) -> Result<Observation, ProbeError> {
    let first = source.first();
    let pids = tasks::resolve_pids(&first, target)?;
    let before = sample(&first, &pids)?;
    let second = source.second();
    let after = sample(&second, &pids)?;
    let secs = source.interval().as_secs_f64().max(f64::EPSILON);

    let ticks_before: HashMap<u32, u64> = before
        .threads
        .iter()
        .map(|(_, t)| (t.tid, t.cpu_ticks))
        .collect();
    let threads = after
        .threads
        .into_iter()
        .filter_map(|(pid, t)| {
            let start = *ticks_before.get(&t.tid)?;
            Some(Thread {
                pid,
                tid: t.tid,
                cpu_share: t.cpu_ticks.saturating_sub(start) as f64 / USER_HZ / secs,
                comm: t.comm,
                core: t.processor,
            })
        })
        .collect();

    let counts_before: HashMap<&str, &BTreeMap<u32, u64>> = before
        .irqs
        .iter()
        .map(|i| (i.label.as_str(), &i.counts))
        .collect();
    let irqs = after
        .irqs
        .iter()
        .filter_map(|line| {
            let start = counts_before.get(line.label.as_str())?;
            let rates = line
                .counts
                .iter()
                .map(|(cpu, n)| {
                    let delta = n.saturating_sub(start.get(cpu).copied().unwrap_or(0));
                    (*cpu, delta as f64 / secs)
                })
                .collect();
            let affinity = line.number().and_then(|n| {
                let list = second
                    .read(&format!("/proc/irq/{n}/smp_affinity_list"))
                    .ok()?;
                CpuSet::parse(&list)
            });
            Some(Irq {
                label: line.label.clone(),
                name: line.name.clone(),
                description: line.description.clone(),
                rates,
                affinity,
            })
        })
        .collect();

    Ok(Observation {
        interval: source.interval(),
        pids,
        threads,
        irqs,
        cpus: cpu::read_cpus(&second),
        isolated: cpu::read_isolated(&second),
        smt_active: cpu::read_smt_active(&second),
        processes: tasks::read_processes(&second),
        tuned_profile: second
            .read("/etc/tuned/active_profile")
            .ok()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty()),
    })
}
