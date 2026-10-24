use std::collections::BTreeSet;

use hpcd_core::{Change, CpuSet, Finding, Plan};
use hpcd_probe::Observation;
use serde::Deserialize;

use crate::checks::{cores_phrase, irq_names, params};
use crate::{Category, Fix, Rule, registry};

#[derive(Debug)]
pub struct Draft {
    pub summary: String,
    pub changes: Vec<Change>,
    pub note: Option<String>,
}

fn housekeeping(obs: &Observation) -> Result<CpuSet, String> {
    let hot = obs.hot_cores();
    let mut busy: BTreeSet<u32> = hot.iter().collect();
    for core in hot.iter() {
        if let Some(siblings) = obs.cpus.get(&core).and_then(|c| c.siblings.as_ref()) {
            busy.extend(siblings.iter());
        }
    }
    let isolated = obs.isolated.clone().unwrap_or_default();
    let free: Vec<u32> = obs
        .cpus
        .keys()
        .copied()
        .filter(|c| !busy.contains(c) && !isolated.contains(*c))
        .collect();
    let nodes: BTreeSet<u32> = hot.iter().filter_map(|c| obs.node_of(c)).collect();
    let near: CpuSet = free
        .iter()
        .copied()
        .filter(|c| obs.node_of(*c).is_some_and(|n| nodes.contains(&n)))
        .collect();
    let cores = if near.is_empty() {
        free.into_iter().collect()
    } else {
        near
    };
    if cores.is_empty() {
        return Err(
            "no housekeeping cores: every core runs a hot thread or its SMT sibling".into(),
        );
    }
    Ok(cores)
}

fn target_cores(to: &str, obs: &Observation) -> Result<CpuSet, String> {
    match to {
        "housekeeping_cores" => housekeeping(obs),
        other => Err(format!("unsupported 'to' value '{other}'")),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToCores {
    to: String,
}

#[derive(Deserialize)]
struct IrqExclude {
    #[serde(default)]
    exclude: Vec<String>,
}

pub fn set_irq_affinity(rule: &Rule, fix: &Fix, obs: &Observation) -> Result<Draft, String> {
    let ToCores { to } = params(&fix.params)?;
    let cpus = target_cores(&to, obs)?;
    let IrqExclude { exclude } = params(&rule.detect.params).unwrap_or(IrqExclude {
        exclude: Vec::new(),
    });
    let hot = obs.hot_cores();
    let irqs: Vec<_> = obs
        .irqs
        .iter()
        .filter(|i| i.number().is_some() && hot.iter().any(|c| i.rate_on(c) > 0.0))
        .filter(|i| !exclude.iter().any(|x| *x == i.label || *x == i.name))
        .collect();
    if irqs.is_empty() {
        return Err("no device IRQs on the hot cores".into());
    }
    let names: Vec<&str> = irqs.iter().map(|i| i.name.as_str()).collect();
    Ok(Draft {
        summary: format!("move {} IRQs to {}", irq_names(&names), cores_phrase(&cpus)),
        changes: irqs
            .iter()
            .filter_map(|i| {
                Some(Change::IrqAffinity {
                    irq: i.number()?,
                    cpus: cpus.clone(),
                })
            })
            .collect(),
        note: None,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HoldDmaLatency {
    value_us: u32,
}

pub fn hold_cpu_dma_latency(_: &Rule, fix: &Fix, _: &Observation) -> Result<Draft, String> {
    let HoldDmaLatency { value_us } = params(&fix.params)?;
    Ok(Draft {
        summary: format!("hold /dev/cpu_dma_latency at {value_us}"),
        changes: vec![Change::CpuDmaLatency { us: value_us }],
        note: Some("applies to every core while held".into()),
    })
}

#[derive(Deserialize)]
struct Governors {
    governors: Vec<String>,
}

pub fn set_cpufreq_governor(rule: &Rule, fix: &Fix, obs: &Observation) -> Result<Draft, String> {
    let ToCores { to } = params(&fix.params)?;
    let Governors { governors } = params(&rule.detect.params)?;
    let cores: CpuSet = obs
        .hot_cores()
        .iter()
        .filter(|c| {
            obs.cpus
                .get(c)
                .and_then(|i| i.governor.as_ref())
                .is_some_and(|g| governors.contains(g))
        })
        .collect();
    if cores.is_empty() {
        return Err("no hot core runs one of the listed governors".into());
    }
    Ok(Draft {
        summary: format!("'{to}' on {}", cores_phrase(&cores)),
        changes: cores
            .iter()
            .map(|cpu| Change::CpuGovernor {
                cpu,
                governor: to.clone(),
            })
            .collect(),
        note: None,
    })
}

#[derive(Deserialize)]
struct MinRate {
    min_rate: f64,
}

pub fn pin_thread(rule: &Rule, fix: &Fix, obs: &Observation) -> Result<Draft, String> {
    let ToCores { to } = params(&fix.params)?;
    if to != "current_core" {
        return Err(format!("unsupported 'to' value '{to}'"));
    }
    let MinRate { min_rate } = params(&rule.detect.params)?;
    let threads: Vec<_> = obs
        .hot_threads()
        .filter(|t| t.migrations_per_sec.is_some_and(|r| r >= min_rate))
        .collect();
    let summary = match threads.as_slice() {
        [] => return Err("no hot thread migrates".into()),
        [t] => format!("pin thread {} to core {}", t.tid, t.core),
        many => format!("pin {} threads to their current cores", many.len()),
    };
    Ok(Draft {
        summary,
        changes: threads
            .iter()
            .map(|t| Change::ThreadAffinity {
                tid: t.tid,
                cpus: CpuSet::from_iter([t.core]),
            })
            .collect(),
        note: None,
    })
}

#[derive(Deserialize)]
struct MinBusy {
    min_busy: f64,
}

pub fn move_tasks_off_cpu(rule: &Rule, fix: &Fix, obs: &Observation) -> Result<Draft, String> {
    let ToCores { to } = params(&fix.params)?;
    let cpus = target_cores(&to, obs)?;
    let MinBusy { min_busy } = params(&rule.detect.params)?;
    let own: CpuSet = obs.threads.iter().map(|t| t.core).collect();
    let busy: CpuSet = obs
        .hot_cores()
        .iter()
        .filter_map(|c| obs.cpus.get(&c)?.siblings.clone())
        .flat_map(|s| s.iter().collect::<Vec<_>>())
        .filter(|s| !own.contains(*s))
        .filter(|s| obs.cpu_busy.get(s).is_some_and(|b| *b >= min_busy))
        .collect();
    if busy.is_empty() {
        return Err("no busy SMT sibling".into());
    }
    Ok(Draft {
        summary: format!("move other work off {}", cores_phrase(&busy)),
        changes: Vec::new(),
        note: Some(format!(
            "movable tasks on {} are listed when the fix is applied and moved to {}",
            cores_phrase(&busy),
            cores_phrase(&cpus)
        )),
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cmdline {
    add: Vec<String>,
}

pub fn propose_kernel_cmdline(_: &Rule, fix: &Fix, obs: &Observation) -> Result<Draft, String> {
    let Cmdline { add } = params(&fix.params)?;
    let hot = obs.hot_cores();
    if hot.is_empty() {
        return Err("no hot cores".into());
    }
    housekeeping(obs).map_err(|e| format!("not proposing isolation: {e}"))?;
    let args: Vec<String> = add
        .iter()
        .map(|a| {
            if a.contains('=') {
                format!("{a},{hot}")
            } else {
                format!("{a}={hot}")
            }
        })
        .collect();
    Ok(Draft {
        summary: format!("add {} to the kernel command line", args.join(" ")),
        changes: Vec::new(),
        note: None,
    })
}

#[derive(Debug)]
pub struct Planned {
    pub rule_id: String,
    pub reboot: bool,
    pub plan: Result<Plan, String>,
}

pub fn plan(rules: &[Rule], findings: &[Finding], obs: &Observation) -> Vec<Planned> {
    let active_conflicts: Vec<&Rule> = rules
        .iter()
        .filter(|r| r.category == Category::Conflicts)
        .filter(|r| findings.iter().any(|f| f.rule_id == r.id))
        .collect();
    let mut matched: Vec<(&Rule, &Fix)> = rules
        .iter()
        .filter(|r| findings.iter().any(|f| f.rule_id == r.id))
        .filter_map(|r| Some((r, r.fix.as_ref()?)))
        .collect();
    matched.sort_by_key(|(r, _)| (r.order, std::cmp::Reverse(r.severity)));
    matched
        .into_iter()
        .map(|(rule, fix)| {
            let plan = registry::action(&fix.action)
                .ok_or_else(|| format!("unknown action '{}'", fix.action))
                .and_then(|a| (a.plan)(rule, fix, obs))
                .map(|draft| Plan {
                    rule_id: rule.id.clone(),
                    summary: draft.summary,
                    changes: draft.changes,
                    risk: fix.risk,
                    reboot: fix.reboot,
                    conflicts: fix
                        .conflicts
                        .iter()
                        .filter(|(tool, _)| {
                            active_conflicts
                                .iter()
                                .any(|c| c.id.starts_with(&format!("{tool}-")))
                        })
                        .map(|(tool, advice)| format!("{tool} is active · {advice}"))
                        .collect(),
                    note: draft.note,
                });
            Planned {
                rule_id: rule.id.clone(),
                reboot: fix.reboot,
                plan,
            }
        })
        .collect()
}
